//! 网络 Agent 的网关（ADR 0010，2-收发）：服务器用 Agent 自己的 Matrix 会话替它同步房间，
//! 用本机 Bridge 同一套解析与验签把消息整理成预览，放进它的收件箱。Agent 凭令牌长轮询取，
//! 只有显式确认才往前走。一个 Agent 同时只有一次长轮询，新来的会让旧的立刻空手返回。
//!
//! 进房间也由网关统筹（第 3 步）：私人房间都是端到端加密的，凭口令放行之后，先把 Agent 切到
//! 加密客户端、建好加密身份，再进去。

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use agent_room_application::{
    content::ContentUseCases,
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentAdmission, NetworkAgentFailure,
        NetworkAgentKnock, NetworkAgentPlacement, NetworkAgentRoom, NetworkAgentRoomRequest,
        NetworkAgentSession, NetworkAgentTarget, NetworkAgentUseCases,
    },
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock, MatrixEventId,
        MatrixFailureKind, MatrixRoomEncryption, MatrixRoomId, MatrixSyncBatch, MatrixSyncToken,
        MatrixUserId, NetworkAgentAckOutcome, NetworkAgentInboxAppend, NetworkAgentInboxChange,
        NetworkAgentInboxPage, NetworkAgentInboxStore, NetworkAgentMatrixGateway,
        NetworkAgentMessageHistory, NetworkAgentMessageRef, NetworkAgentSubmissionStore,
        NetworkAgentSyncRequest, PortFuture,
    },
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    agent_verification::{
        AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
    },
    matrix_security::MatrixSecurityFailure,
    messages::{
        MessageBody, MessageEventPublisher, MessageProjectionBatch, MessagePublicationDependencies,
        MessagePublicationFailure, MessagePublicationFailureKind, MessagePublicationOutcome,
        MessagePublicationService, MessageStoreFailureKind, MessageSyncDependencies,
        MessageSyncService, ProtectMessageBodyFailureKind, ProtectMessageBodyRequest,
        SendMessageRequest,
    },
    status::{AgentStatusIntent, HostAgentState},
};
use agent_room_bridge_ipc::{
    IpcMessagePreviewSummary, IpcTimelineGap,
    previews::{PreviewViewer, truncate_preview_value},
    wake::{
        Arrival, Delivery, IpcWake, WaitDecision, WaitOptions, WakeContext, WakeReason, WakeRule,
        decide,
    },
};
use agent_room_domain::{
    content::{ContentEncryptionMode, ContentMediaType},
    ids::{AgentId, AutomationGrantId, MessageId, MessageSubmissionId, NetworkAgentId},
    messages::{
        ConversationMessage, MessagePreview, MessageProvenance, MessageRelation, MessageRiskFlags,
        MessageSensitivity, MessageSummary, MessageTitle,
    },
    time::{DurationMillis, UtcMillis},
};
use serde::Deserialize;
use serde_json::Value;
use tokio::{sync::Notify, time::Instant};
use uuid::{Uuid, Version};

pub(crate) use cleanup::NetworkAgentCleanupOutcome;
pub(crate) use encrypted::{EncryptedClients, EncryptedSessions, EncryptedSpeaker};
pub(crate) use viewing::{
    NetworkAgentFoundMessages, NetworkAgentRoomMessages, NetworkAgentRoomMessagesRequest,
    NetworkAgentRoomQuery,
};

mod backfill;
mod before_join;
mod cleanup;
mod encrypted;
mod presence;
mod projection;
mod speaking;
#[cfg(test)]
mod tests;
mod typing;
mod viewing;

/// 长轮询最多等这么久。
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(30);
/// 一次最多取这么多条。
pub(crate) const MAX_PAGE: u16 = 50;
/// 第一次同步只带每个房间最近几条，给 Agent 一点上下文。
const FIRST_SYNC_TIMELINE_LIMIT: u16 = 20;
const SYNC_TIMELINE_LIMIT: u16 = 50;
/// 每个房间最多留这么多条没确认的；再多就丢掉这个房间最早的，并在下次取消息时告诉 Agent 丢了几条。
pub(crate) const INBOX_CAPACITY: u32 = 500;
/// 消息记录每个房间留最近这么多条（确认过的、它自己发的也算），按需查看时能翻到这么远
/// （维护者 2026-09-30 定）。
pub(crate) const HISTORY_CAPACITY: u32 = 500;
/// 等消息时一次看这么多条没确认的，按规则挑出要交的。
const INBOX_PAGE: u16 = 200;
/// 长轮询分段等，每段不超过这么久：到了续租时间，等待中也能续上。
const SYNC_CHUNK: Duration = Duration::from_secs(10);
/// 聊天正文的媒体类型，与 MCP 的聊天发言一致。
const CHAT_MEDIA_TYPE: &str = "text/plain";
/// 摘要取正文压缩空白后的前 500 个字符，标题再取摘要的前 120 个，与 MCP 的聊天发言一致。
const SUMMARY_CHARACTERS: usize = 500;
const TITLE_CHARACTERS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkGatewayFailure {
    /// 令牌、总开关、限流这类问题，按网络 Agent 的错误码回答。
    Agent(NetworkAgentFailure),
    /// Matrix 或数据库暂时不可用，可以原样重试；发言时带同一个 submissionId 重试不会重复。
    Unavailable,
    /// 确认的事件 ID 格式不对。
    InvalidEvent,
    /// 发言的内容不合规：说明是哪一项。
    InvalidMessage(&'static str),
    /// 等消息的参数不对：说明是哪一项。
    InvalidWait(&'static str),
    /// 按需查看的参数不对：说明是哪一项。
    InvalidLookup(&'static str),
    /// 这个房间的消息记录里没有这一条（看前后、翻页的位置）。
    MessageNotFound,
    /// 在不止一个房间里却没说发到哪间。
    RoomRequired,
    /// 不在这个房间里。
    RoomNotJoined,
    /// 这个 submissionId 已经用来发过别的内容。
    SubmissionConflict,
    /// 内容服务拒绝了这条发言（例如已经不是房间成员）。
    Forbidden,
    Internal,
}

/// 网络 Agent 要发的一条聊天。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkAgentMessageDraft {
    /// Matrix 房间 ID；只在一个房间里时可以省略。
    pub(crate) room_id: Option<String>,
    pub(crate) text: String,
    /// 回复的那条消息的 messageId。
    pub(crate) reply_to: Option<String>,
    /// 提及的 Matrix 用户，最多 200 个。
    pub(crate) mentions: Vec<String>,
    /// @所有人：只能发到私人房间（端到端加密的房间）。
    pub(crate) mentions_everyone: bool,
    /// 幂等标识（UUIDv7）；不带就由服务器生成并返回，重试时带上。
    pub(crate) submission_id: Option<String>,
}

/// 发言结果：`event` 为空时说明 Matrix 还没确认，带同一个 submissionId 重试即可。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkAgentSentMessage {
    pub(crate) submission: MessageSubmissionId,
    pub(crate) room: String,
    pub(crate) event: Option<String>,
}

/// 一次取到的消息：最早的在前。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NetworkAgentMessages {
    pub(crate) messages: Vec<Value>,
    /// 还没确认的总数，可能多于这一次取到的。
    pub(crate) pending: u64,
    /// 收件箱满了丢掉的条数，确认之后清零。
    pub(crate) dropped: u64,
    /// 为什么这时候交：被什么叫醒、叫醒它的是哪几条、等齐时谁还没说话。
    pub(crate) wake: IpcWake,
    /// 交出去的最后一条之前没交的条数；确认到最后一条时，它们也算看过。
    pub(crate) skipped: u64,
    /// 交出去的最后一条之后还没确认的条数，下次再给。
    pub(crate) remaining: u64,
    /// 交出去的这些消息前面补不回来的几段：一次来得太多、往回补到上限还没接上。
    pub(crate) gaps: Vec<IpcTimelineGap>,
}

/// 一次等消息：最多等多久、一次最多取几条、听什么（`specs/agent-reading/waiting.md`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkAgentWait {
    pub(crate) wait: Duration,
    pub(crate) limit: u16,
    pub(crate) options: WaitOptions,
    /// `waitFor=mentioned`：等我上一条点到的人。
    pub(crate) wait_for_mentioned: bool,
    /// 只看这个房间（Matrix 房间 ID）；确认时也带上它，就只确认这个房间的。
    pub(crate) room: Option<String>,
}

/// 已有的网络 Agent 再进一个房间的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkAgentEntry {
    /// 进去了，或者本来就在里面。
    Entered(NetworkAgentRoom),
    /// 拿私人房间的房间号敲了门，等管理者放行（`specs/network-agents/knock.md`）。
    Knocked(NetworkAgentKnock),
}

/// 网络 Agent 进房间与收发消息的接口；HTTP 接口与远程 MCP 只认这个，便于单独测试。
pub(crate) trait NetworkAgentMessaging: Send + Sync {
    /// 起名并进房间：公开大厅直接进；凭口令的私人房间先让加密客户端就绪再进；拿私人房间的房间号
    /// 只敲门。进不去时停用刚建的人物，令牌不交出去。
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, Result<CreatedNetworkAgent, NetworkGatewayFailure>>;

    /// 已有的网络 Agent 再进一个房间；已经在里面就原样返回，拿私人房间的房间号就敲门。
    fn enter_room<'a>(
        &'a self,
        token: &'a str,
        room: NetworkAgentRoomRequest,
        source_digest: [u8; 32],
    ) -> PortFuture<'a, Result<NetworkAgentEntry, NetworkGatewayFailure>>;

    /// 等消息：按 `request` 的规则等到有事，防抖后交出去；等满时间就空手返回。
    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        request: NetworkAgentWait,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>>;

    /// 确认处理到这一条（含）为止。给了 `room` 就只确认这个房间的，不给就是所有房间里在它
    /// 之前到的。
    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
        room: Option<&'a str>,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>>;

    /// 按 ID 取消息的全文，不动收件箱。
    fn get_messages<'a>(
        &'a self,
        token: &'a str,
        ids: Vec<String>,
    ) -> PortFuture<'a, Result<NetworkAgentFoundMessages, NetworkGatewayFailure>>;

    /// 看一条消息的前后，或者往前（往后）翻，不动收件箱。
    fn room_messages<'a>(
        &'a self,
        token: &'a str,
        request: NetworkAgentRoomMessagesRequest,
    ) -> PortFuture<'a, Result<NetworkAgentRoomMessages, NetworkGatewayFailure>>;

    fn send_message<'a>(
        &'a self,
        token: &'a str,
        draft: NetworkAgentMessageDraft,
    ) -> PortFuture<'a, Result<NetworkAgentSentMessage, NetworkGatewayFailure>>;

    /// 离开所有房间并作废令牌。
    fn leave_and_disable<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>>;
}

/// 管理者放行敲门后，替网络 Agent 进那个私人房间（`specs/network-agents/knock.md` 的“放行”）。
/// 管理者的接口只认这个，便于单独测试。
pub(crate) trait AdmittedAgentEntry: Send + Sync {
    /// 按 Agent ID 取生效中的网络 Agent 的会话，和它自己凭口令进私人房间一样进去。
    /// 已经在里面也照样成功。
    fn enter_admitted(
        &self,
        agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> PortFuture<'_, Result<NetworkAgentRoom, NetworkGatewayFailure>>;
}

pub(crate) struct NetworkGatewayDependencies {
    pub(crate) agents: Arc<dyn NetworkAgentUseCases>,
    pub(crate) inbox: Arc<dyn NetworkAgentInboxStore>,
    pub(crate) history: Arc<dyn NetworkAgentMessageHistory>,
    pub(crate) submissions: Arc<dyn NetworkAgentSubmissionStore>,
    pub(crate) matrix: Arc<dyn NetworkAgentMatrixGateway>,
    pub(crate) content: Arc<dyn ContentUseCases>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) clock: Arc<dyn Clock>,
    /// 进过加密房间的 Agent 用的 matrix-sdk 客户端；总开关关着时没有。
    pub(crate) encrypted: Option<Arc<dyn EncryptedSessions>>,
}

pub(crate) struct NetworkGateway {
    agents: Arc<dyn NetworkAgentUseCases>,
    inbox: Arc<dyn NetworkAgentInboxStore>,
    history: Arc<dyn NetworkAgentMessageHistory>,
    submissions: Arc<dyn NetworkAgentSubmissionStore>,
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    content: Arc<dyn ContentUseCases>,
    verification: Arc<dyn AgentInstanceVerificationRepository>,
    signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    clock: Arc<dyn Clock>,
    encrypted: Option<Arc<dyn EncryptedSessions>>,
    /// 这个进程刚切到加密客户端的：切之前取的会话里还没有 `encrypted_since`。
    switched: Mutex<HashSet<NetworkAgentId>>,
    polls: LongPolls,
    presence: Arc<presence::Presence>,
    /// 每个网络 Agent 上一条发言点到的人，`waitFor=mentioned` 用。只记在这个进程里：
    /// 重启以后要它直接写 Matrix 用户 ID。
    last_mentions: Mutex<HashMap<NetworkAgentId, Vec<String>>>,
    /// 每个网络 Agent 的房间里此刻谁在打字。
    typing: typing::AgentTyping,
}

impl NetworkGateway {
    pub(crate) fn new(dependencies: NetworkGatewayDependencies) -> Self {
        Self {
            agents: dependencies.agents,
            inbox: dependencies.inbox,
            history: dependencies.history,
            submissions: dependencies.submissions,
            matrix: dependencies.matrix,
            content: dependencies.content,
            verification: dependencies.verification,
            signatures: dependencies.signatures,
            clock: dependencies.clock,
            encrypted: dependencies.encrypted,
            switched: Mutex::new(HashSet::new()),
            polls: LongPolls::default(),
            presence: Arc::default(),
            last_mentions: Mutex::new(HashMap::new()),
            typing: typing::AgentTyping::default(),
        }
    }

    /// 以这个 Agent 的身份发一条聊天：限流后交给本机 Bridge 同一个发布服务。
    async fn send_internal(
        &self,
        token: &str,
        draft: NetworkAgentMessageDraft,
    ) -> Result<NetworkAgentSentMessage, NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let room = session_room(&session, draft.room_id.as_deref())?;
        let submission_id = submission_id(draft.submission_id.as_deref())?;
        // 进过加密房间的由它的加密客户端发出：加密房间里正文先加密，事件由客户端加密。
        let speaker = if self.is_encrypted(&session) {
            Some(self.encrypted_speaker(&session, &room).await?)
        } else {
            None
        };
        everyone_allowed(draft.mentions_everyone, speaker.as_ref())?;
        let body = chat_body(submission_id, &room, speaker.as_ref(), &draft.text)?;
        let mentions = draft.mentions.clone();
        let request = chat_request(&session, submission_id, room.clone(), draft, body)?;
        self.agents
            .take_message_quota(session.network_agent_id)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let signer = speaking::SessionSigner::from_seed(&session.instance_signing_seed)
            .ok_or(NetworkGatewayFailure::Internal)?;
        let identity = BridgeAgentIdentity::new(
            session.agent_id,
            session.display_name.clone(),
            session.agent_matrix_user_id.clone(),
            session.agent_instance_id,
        )
        .map_err(|_| NetworkGatewayFailure::Internal)?;
        let catalog_id = session
            .rooms
            .iter()
            .find(|joined| joined.matrix_room_id.as_str() == room.as_str())
            .map(|joined| joined.catalog_id)
            .ok_or(NetworkGatewayFailure::RoomNotJoined)?;
        let publisher: Arc<dyn MessageEventPublisher> = match speaker {
            Some((speaker, _)) => Arc::new(speaking::ClientPublisher {
                matrix: speaker.matrix,
            }),
            None => Arc::new(speaking::SessionPublisher {
                matrix: self.matrix.clone(),
                access_token: session.matrix_access_token.clone(),
            }),
        };
        let publication = MessagePublicationService::new(MessagePublicationDependencies {
            identity,
            signer: Arc::new(signer),
            publisher,
            content: Arc::new(speaking::InProcessContent {
                content: self.content.clone(),
                principal_id: session.principal_id,
                agent_id: session.agent_id,
            }),
            submissions: Arc::new(self.submissions_for(&session)),
            automation: Arc::new(speaking::QuotaAlreadyTaken),
            room_catalog_id: catalog_id,
        });
        let event_id = match publication.send(&request).await {
            Ok(MessagePublicationOutcome::Published { event_id, .. }) => {
                Some(event_id.as_str().to_owned())
            }
            // Matrix 可能已经收下只是没回话，或正文还没绑定：带同一个 submissionId 重试即可。
            Ok(
                MessagePublicationOutcome::PendingReconciliation { .. }
                | MessagePublicationOutcome::AcceptedBindingPending { .. },
            ) => None,
            Err(failure) => return Err(publication_failure(failure, self.clock.now())),
        };
        self.last_mentions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session.network_agent_id, mentions);
        Ok(NetworkAgentSentMessage {
            submission: submission_id,
            room: room.as_str().to_owned(),
            event: event_id,
        })
    }

    /// 尽量离开所有房间，再作废令牌。离开失败不挡作废：令牌作废才是停用的关键，
    /// 没离开成的房间由定时清理补上。
    async fn leave_and_disable_internal(&self, token: &str) -> Result<(), NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let left = self.leave_rooms(&session).await;
        self.agents
            .disable(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        self.presence.forget(session.network_agent_id).await;
        self.typing.forget(session.network_agent_id);
        self.close_encrypted(session.network_agent_id).await;
        if left
            && self
                .agents
                .mark_rooms_left(session.network_agent_id)
                .await
                .is_err()
        {
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                "网络 Agent 已经离开房间但没记下来，定时清理会再确认一次"
            );
        }
        Ok(())
    }

    /// 先说一声下线（离开之后就写不了房间状态了），再离开每个房间。
    /// 都离开了（或本来就不在里面）才返回 true。
    async fn leave_rooms(&self, session: &NetworkAgentSession) -> bool {
        self.presence
            .publish(
                &self.matrix,
                &self.clock,
                session,
                &AgentStatusIntent::new(HostAgentState::Disconnected, None),
            )
            .await;
        let mut left = true;
        for room in &session.rooms {
            if let Ok(room_id) = MatrixRoomId::new(room.matrix_room_id.as_str())
                && self
                    .matrix
                    .leave(&session.matrix_access_token, &room_id)
                    .await
                    .is_err()
            {
                tracing::warn!(
                    network_agent.id = %session.network_agent_id,
                    "网络 Agent 没能离开房间，稍后由定时清理再试"
                );
                left = false;
            }
        }
        left
    }

    fn submissions_for(&self, session: &NetworkAgentSession) -> speaking::AgentSubmissions {
        speaking::AgentSubmissions {
            store: self.submissions.clone(),
            id: session.network_agent_id,
            clock: self.clock.clone(),
        }
    }

    /// 等消息（`specs/agent-reading/waiting.md`）：按 `request` 的规则等到有事，防抖后交出去；
    /// 等满时间就空手返回，没叫醒它的消息留在收件箱里。
    async fn wait_internal(
        &self,
        token: &str,
        request: NetworkAgentWait,
    ) -> Result<NetworkAgentMessages, NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let options = self.wait_options(&session, &request)?;
        let peek_options = peek(&options);
        let room = scoped_room(&session, request.room.as_deref())?;
        let poll = self.polls.begin(session.network_agent_id);
        let mut waiting =
            presence::WaitAnnouncement::new(&self.presence, &self.matrix, &self.clock, &session);
        let wait = request.wait.min(MAX_WAIT);
        let deadline = Instant::now() + wait;
        let deadline_ms = self.clock.now().value().saturating_add(millis(wait));
        let limit = usize::from(request.limit.clamp(1, MAX_PAGE));
        let direct_rooms = HashSet::new();
        // 每次取消息至少同步一次。原来收件箱里还有没确认的、或者 wait=0 时就不问 Matrix：
        // Agent 处理得慢时服务器停着不取，等它确认完，一个房间里积下的超过一次同步能带回的
        // 条数，更早的就悄悄丢了；只用 wait=0 轮询的 Agent 第一次之后再也收不到新消息。
        let mut synced = false;
        // 等 0 秒、或者第一次取消息（带回房间里最近几条当上下文）时，有什么给什么。
        let mut peek = wait.is_zero();
        loop {
            let page = self
                .inbox
                .pending(session.network_agent_id, room.as_ref(), INBOX_PAGE)
                .await
                .map_err(|_| NetworkGatewayFailure::Unavailable)?;
            let first = page.sync_token.is_none();
            peek |= first;
            let now_ms = self.clock.now().value();
            let rules = if peek { &peek_options } else { &options };
            let typing = self.typing.typists(session.network_agent_id, now_ms);
            let context = WakeContext {
                owner: None,
                direct_rooms: &direct_rooms,
                typing: &typing,
            };
            let (ready, recheck_at_ms) =
                match judge(&page, rules, context, limit, now_ms, deadline_ms) {
                    WaitDecision::Deliver(delivery) if synced => {
                        return Ok(delivered(page, delivery));
                    }
                    WaitDecision::Deliver(delivery) => (Some(delivery), None),
                    WaitDecision::Wait { recheck_at_ms } => (None, recheck_at_ms),
                };
            let remaining = deadline.saturating_duration_since(Instant::now());
            // 只在没有可交的、又还有时间时才等新消息，最多等到防抖或定时看一眼到点；
            // 第一次同步只把房间里最近的几条拿来当上下文，不等。
            let block = !first && ready.is_none() && !remaining.is_zero();
            let chunk = recheck_at_ms
                .map_or(remaining, |at_ms| remaining.min(until(now_ms, at_ms)))
                .min(SYNC_CHUNK);
            if block {
                waiting.announce().await;
            }
            let sync_request = NetworkAgentSyncRequest {
                since: page.sync_token.clone(),
                timeout_millis: if block {
                    u64::try_from(chunk.as_millis()).unwrap_or(u64::MAX)
                } else {
                    0
                },
                timeline_limit: if first {
                    FIRST_SYNC_TIMELINE_LIMIT
                } else {
                    SYNC_TIMELINE_LIMIT
                },
            };
            let started = Instant::now();
            let batch = tokio::select! {
                result = self.sync_and_backfill(&session, &sync_request, first) => match (result, ready) {
                    (Ok(batch), _) => batch,
                    // 本来就有可交的：这次没同步成也先交出去，下次再取。
                    (Err(_), Some(delivery)) => return Ok(delivered(page, delivery)),
                    (Err(failure), None) => return Err(failure),
                },
                () = poll.superseded() => return Ok(nothing(&page, WakeReason::Superseded)),
            };
            let elapsed_ms = started.elapsed().as_millis();
            self.typing.record(
                session.network_agent_id,
                &batch.batch,
                self.clock.now().value(),
            );
            self.store_batch(&session, page.sync_token, &sync_request, &batch, elapsed_ms)
                .await?;
            synced = true;
        }
    }

    /// `waitFor=mentioned` 换成上一条点到的人；没发过言、或者上一条没点人时说不清等谁。
    fn wait_options(
        &self,
        session: &NetworkAgentSession,
        request: &NetworkAgentWait,
    ) -> Result<WaitOptions, NetworkGatewayFailure> {
        let mut options = request.options.clone();
        if request.wait_for_mentioned {
            let mentioned = self
                .last_mentions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&session.network_agent_id)
                .cloned()
                .unwrap_or_default();
            if mentioned.is_empty() {
                return Err(NetworkGatewayFailure::InvalidWait("waitFor"));
            }
            options.wait_for = mentioned;
        }
        Ok(options)
    }

    /// 把一次同步的结果写进收件箱。位置对不上说明另一次同步已经写过（例如被取代的旧长轮询），
    /// 下一轮从新位置接着来。
    async fn store_batch(
        &self,
        session: &NetworkAgentSession,
        since: Option<MatrixSyncToken>,
        request: &NetworkAgentSyncRequest,
        backfilled: &backfill::Backfilled,
        elapsed_ms: u128,
    ) -> Result<(), NetworkGatewayFailure> {
        let batch = &backfilled.batch;
        let mut changes = self.changes(session, batch).await?;
        backfill::mark_losses(&mut changes, &backfilled.losses);
        let undecryptable_before_join =
            before_join::undecryptable_before_join(batch, &session.agent_matrix_user_id);
        if !undecryptable_before_join.is_empty() {
            tracing::info!(
                network_agent.id = %session.network_agent_id,
                rooms = undecryptable_before_join.len(),
                "网络 Agent 刚进的加密房间里，加入之前的消息它解不开；交出这个房间下一条消息时告诉它"
            );
        }
        let change_count = changes.len();
        let outcome = self
            .inbox
            .append(&NetworkAgentInboxAppend {
                id: session.network_agent_id,
                expected_sync_token: since.clone(),
                next_sync_token: batch.next_batch().clone(),
                changes,
                received_at: self.clock.now(),
                capacity: INBOX_CAPACITY,
                history_capacity: HISTORY_CAPACITY,
                undecryptable_before_join,
            })
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        tracing::debug!(
            network_agent.id = %session.network_agent_id,
            since = ?since,
            next = ?batch.next_batch(),
            changes = change_count,
            outcome = ?outcome,
            timeout_ms = request.timeout_millis,
            elapsed_ms,
            rooms = batch.rooms().len(),
            timeline_events = batch.rooms().iter().map(|room| room.timeline().len()).sum::<usize>(),
            "网络 Agent 长轮询同步了一段"
        );
        Ok(())
    }

    /// 起名并进房间。凭口令的私人房间只放行了：先切到加密客户端再进；进不去就停用刚建的人物。
    /// 只敲了门的不在任何房间里，等管理者放行。
    async fn create_internal(
        &self,
        request: CreateNetworkAgent,
    ) -> Result<CreatedNetworkAgent, NetworkGatewayFailure> {
        let mut created = self
            .agents
            .create(request)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let admitted = match &created.placement {
            NetworkAgentPlacement::Entered(_) => {
                self.announce_online(created.token.expose()).await;
                return Ok(created);
            }
            NetworkAgentPlacement::Knocked(_) => return Ok(created),
            NetworkAgentPlacement::Admitted(room) => room.clone(),
        };
        match self.enter_private(created.token.expose(), admitted).await {
            Ok(room) => {
                created.placement = NetworkAgentPlacement::Entered(room);
                self.announce_online(created.token.expose()).await;
                Ok(created)
            }
            Err(failure) => {
                // 令牌不会交给 Agent，这个人物也就没用了：停用它，放开名字与全站名额。
                if self.agents.disable(created.token.expose()).await.is_err() {
                    tracing::warn!(
                        network_agent.id = %created.network_agent_id,
                        "凭口令创建时没进去房间，停用也没成；它不会再有活动，闲置清理会停用它"
                    );
                }
                self.close_encrypted(created.network_agent_id).await;
                Err(failure)
            }
        }
    }

    async fn enter_room_internal(
        &self,
        token: &str,
        room: NetworkAgentRoomRequest,
        source_digest: [u8; 32],
    ) -> Result<NetworkAgentEntry, NetworkGatewayFailure> {
        match self
            .agents
            .admit(token, room, source_digest)
            .await
            .map_err(NetworkGatewayFailure::Agent)?
        {
            NetworkAgentAdmission::AlreadyIn(room) => Ok(NetworkAgentEntry::Entered(room)),
            NetworkAgentAdmission::Knocked(knock) => Ok(NetworkAgentEntry::Knocked(knock)),
            NetworkAgentAdmission::Admitted(NetworkAgentTarget::Private(room)) => {
                let room = self.enter_private(token, room).await?;
                self.announce_online(token).await;
                Ok(NetworkAgentEntry::Entered(room))
            }
            NetworkAgentAdmission::Admitted(target) => {
                let room = self
                    .agents
                    .enter(token, target)
                    .await
                    .map_err(NetworkGatewayFailure::Agent)?;
                if let Ok(session) = self.agents.session(token).await
                    && self.is_encrypted(&session)
                {
                    self.refresh_encrypted(&session).await;
                }
                self.announce_online(token).await;
                Ok(NetworkAgentEntry::Entered(room))
            }
        }
    }

    /// 管理者放行敲门后替网络 Agent 进房间：和它自己凭口令进私人房间一样，先切到加密客户端、
    /// 建好加密身份再进，进了说一声在线。切过去时它正在等的消息立刻空手返回，再等就从这个房间收。
    async fn enter_admitted_internal(
        &self,
        agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> Result<NetworkAgentRoom, NetworkGatewayFailure> {
        let session = self
            .agents
            .session_of_agent(agent_id)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        self.switch_to_encrypted(&session).await?;
        let room = self
            .agents
            .enter_admitted(agent_id, room)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        // 进了以后再取一次会话：在线状态要发到新进的这个房间里。
        if let Ok(session) = self.agents.session_of_agent(agent_id).await {
            self.refresh_encrypted(&session).await;
            self.publish_online(&session).await;
        }
        Ok(room)
    }

    /// 进了房间先说一声在线：网页的成员栏按 Agent 发布的状态列出 Agent，不说的话，人要等它
    /// 第一次收消息时才看得到它（2026-10-02 维护者遇到过）。没说成只记日志，不影响进房间。
    async fn announce_online(&self, token: &str) {
        let Ok(session) = self.agents.session(token).await else {
            return;
        };
        self.publish_online(&session).await;
    }

    async fn publish_online(&self, session: &NetworkAgentSession) {
        self.presence
            .publish(
                &self.matrix,
                &self.clock,
                session,
                &AgentStatusIntent::new(HostAgentState::Available, None),
            )
            .await;
    }

    /// 私人房间都是端到端加密的：先切到加密客户端、建好加密身份，再进去。房间密钥只发给由
    /// 主人交叉签名的设备，身份没建好就先进，别人这时发的消息它会解不开。
    async fn enter_private(
        &self,
        token: &str,
        room: NetworkAgentRoom,
    ) -> Result<NetworkAgentRoom, NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        self.switch_to_encrypted(&session).await?;
        let room = self
            .agents
            .enter(token, NetworkAgentTarget::Private(room))
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        self.refresh_encrypted(&session).await;
        Ok(room)
    }

    /// 进了房间之后让加密客户端完整同步一次，马上认识这个房间，进来就能发言。没同步成不要紧：
    /// 发言时发现它还不认识这个房间会再同步。
    async fn refresh_encrypted(&self, session: &NetworkAgentSession) {
        if let Some(encrypted) = &self.encrypted
            && let Err(failure) = encrypted.refresh(session).await
        {
            tracing::warn!(
                network_agent.id = %session.network_agent_id,
                failure = ?failure,
                "进房间后加密客户端没同步成，发言时再同步"
            );
        }
    }

    /// 以加密客户端发言之前：确认还在这个房间里，看它加不加密。加密房间先确认自己的身份就绪、
    /// 刷新成员身份；客户端还不认识这个房间（刚进来还没同步到）时完整同步一次再试。
    async fn encrypted_speaker(
        &self,
        session: &NetworkAgentSession,
        room: &MatrixRoomId,
    ) -> Result<(EncryptedSpeaker, MatrixRoomEncryption), NetworkGatewayFailure> {
        let Some(encrypted) = &self.encrypted else {
            return Err(NetworkGatewayFailure::Unavailable);
        };
        let speaker = encrypted.speaker(session).await?;
        let user = MatrixUserId::new(session.agent_matrix_user_id.clone())
            .map_err(|_| NetworkGatewayFailure::Internal)?;
        let authority = speaker
            .authority
            .inspect_room_authority(room, &user)
            .await
            .map_err(|failure| {
                if failure.kind() == MatrixFailureKind::Forbidden {
                    NetworkGatewayFailure::RoomNotJoined
                } else {
                    NetworkGatewayFailure::Unavailable
                }
            })?;
        if !authority.is_joined() {
            return Err(NetworkGatewayFailure::RoomNotJoined);
        }
        let encryption = authority.encryption();
        if encryption == MatrixRoomEncryption::EndToEnd {
            let ready = match speaker.security.ensure_room_ready(room).await {
                Err(MatrixSecurityFailure::NotJoined) => {
                    encrypted.refresh(session).await?;
                    speaker.security.ensure_room_ready(room).await
                }
                ready => ready,
            };
            ready.map_err(|failure| match failure {
                MatrixSecurityFailure::NotJoined => NetworkGatewayFailure::RoomNotJoined,
                MatrixSecurityFailure::IdentityChanged => NetworkGatewayFailure::Forbidden,
                _ => NetworkGatewayFailure::Unavailable,
            })?;
        }
        Ok((speaker, encryption))
    }

    /// 切到加密客户端。先在内存里记下，并让正在进行的长轮询立刻返回：之后的同步都走加密客户端，
    /// 轻量客户端不会再把发给这台设备的房间密钥跳过去。再记入库，让加密客户端完整同步一次
    /// （认识所有已加入的房间，上传设备密钥与一次性密钥），并建好加密身份与密钥备份。
    async fn switch_to_encrypted(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<(), NetworkGatewayFailure> {
        let mut session = session.clone();
        let id = session.network_agent_id;
        let Some(encrypted) = &self.encrypted else {
            tracing::error!(
                network_agent.id = %id,
                "要进加密房间，但加密客户端没有配置"
            );
            return Err(NetworkGatewayFailure::Unavailable);
        };
        self.switched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id);
        self.polls.supersede(id);
        if session.encrypted_since.is_none() {
            session.encrypted_since = Some(
                self.agents
                    .mark_encrypted(id)
                    .await
                    .map_err(NetworkGatewayFailure::Agent)?,
            );
        }
        encrypted.prepare(&session).await
    }

    /// 进过加密房间：库里记过，或这个进程刚把它切过去（会话是切之前取的）。
    fn is_encrypted(&self, session: &NetworkAgentSession) -> bool {
        session.encrypted_since.is_some()
            || self
                .switched
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&session.network_agent_id)
    }

    /// 进过加密房间的 Agent 由它的 matrix-sdk 客户端同步，其余的走轻量客户端。
    async fn sync(
        &self,
        session: &NetworkAgentSession,
        request: &NetworkAgentSyncRequest,
    ) -> Result<MatrixSyncBatch, NetworkGatewayFailure> {
        if !self.is_encrypted(session) {
            return self
                .matrix
                .sync(&session.matrix_access_token, request)
                .await
                .map_err(|_| NetworkGatewayFailure::Unavailable);
        }
        // 不能退回轻量客户端：它同步时会把发给这台设备的房间密钥一并跳过。
        let Some(encrypted) = &self.encrypted else {
            tracing::error!(
                network_agent.id = %session.network_agent_id,
                "网络 Agent 进过加密房间，但加密客户端没有配置"
            );
            return Err(NetworkGatewayFailure::Unavailable);
        };
        encrypted.sync(session, request).await
    }

    /// 停用后关掉它的加密客户端（如果开着）。
    async fn close_encrypted(&self, id: NetworkAgentId) {
        if let Some(encrypted) = &self.encrypted {
            encrypted.forget(id).await;
        }
    }

    /// 确认处理到这一条（含）为止；给了房间就只确认这个房间的。
    async fn acknowledge_internal(
        &self,
        token: &str,
        event_id: &str,
        room: Option<&str>,
    ) -> Result<NetworkAgentAckOutcome, NetworkGatewayFailure> {
        let event_id =
            MatrixEventId::new(event_id).map_err(|_| NetworkGatewayFailure::InvalidEvent)?;
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let room = scoped_room(&session, room)?;
        self.inbox
            .acknowledge(session.network_agent_id, &event_id, room.as_ref())
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)
    }

    /// 用本机 Bridge 的同步服务解析、验签这一批，只截下结果，不落它的投影。
    async fn changes(
        &self,
        session: &NetworkAgentSession,
        batch: &MatrixSyncBatch,
    ) -> Result<Vec<NetworkAgentInboxChange>, NetworkGatewayFailure> {
        let captured = Arc::new(projection::CapturedProjection::default());
        let sync = MessageSyncService::new(MessageSyncDependencies {
            authenticator: Arc::new(AgentInstanceMessageAuthenticator::new(
                AgentInstanceMessageAuthenticatorDependencies {
                    verification: Arc::new(projection::InstanceVerification::new(
                        self.verification.clone(),
                    )),
                    signatures: self.signatures.clone(),
                },
            )),
            projections: captured.clone(),
            // 自己发出去、还不知道 Matrix 收没收到的，同步时按事务 ID 对上。
            submissions: Arc::new(self.submissions_for(session)),
        });
        sync.process(batch)
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        let projected = captured.take();
        let stored = self.stored_replies(session, projected.as_ref()).await;
        Ok(projection::inbox_changes(
            projected,
            PreviewViewer {
                agent_id: session.agent_id,
                matrix_user_id: &session.agent_matrix_user_id,
            },
            &session.rooms,
            &stored,
        ))
    }

    /// 被回复的那条不在这一批里时，去消息记录里找。找不到、读不出来只是少了摘录。
    async fn stored_replies(
        &self,
        session: &NetworkAgentSession,
        batch: Option<&MessageProjectionBatch>,
    ) -> projection::StoredReplies {
        let targets = batch.map(projection::reply_targets).unwrap_or_default();
        if targets.is_empty() {
            return projection::StoredReplies::new();
        }
        let refs: Vec<NetworkAgentMessageRef> = targets
            .iter()
            .map(|(_, id)| NetworkAgentMessageRef::Message(*id))
            .collect();
        let found = match self
            .history
            .messages_by_id(session.network_agent_id, &refs)
            .await
        {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(
                    network_agent.id = %session.network_agent_id,
                    error = ?error,
                    "网络 Agent 的消息记录读不出来，这一批回复的是哪条只能先不写"
                );
                return projection::StoredReplies::new();
            }
        };
        found
            .into_iter()
            .filter_map(|message| {
                let summary: IpcMessagePreviewSummary =
                    serde_json::from_value(message.preview).ok()?;
                Some(((message.room_id, message.message_id), summary))
            })
            .collect()
    }
}

impl AdmittedAgentEntry for NetworkGateway {
    fn enter_admitted(
        &self,
        agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> PortFuture<'_, Result<NetworkAgentRoom, NetworkGatewayFailure>> {
        Box::pin(self.enter_admitted_internal(agent_id, room))
    }
}

impl NetworkAgentMessaging for NetworkGateway {
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, Result<CreatedNetworkAgent, NetworkGatewayFailure>> {
        Box::pin(self.create_internal(request))
    }

    fn enter_room<'a>(
        &'a self,
        token: &'a str,
        room: NetworkAgentRoomRequest,
        source_digest: [u8; 32],
    ) -> PortFuture<'a, Result<NetworkAgentEntry, NetworkGatewayFailure>> {
        Box::pin(self.enter_room_internal(token, room, source_digest))
    }

    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        request: NetworkAgentWait,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>> {
        Box::pin(self.wait_internal(token, request))
    }

    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
        room: Option<&'a str>,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>> {
        Box::pin(self.acknowledge_internal(token, event_id, room))
    }

    fn get_messages<'a>(
        &'a self,
        token: &'a str,
        ids: Vec<String>,
    ) -> PortFuture<'a, Result<NetworkAgentFoundMessages, NetworkGatewayFailure>> {
        Box::pin(self.get_messages_internal(token, ids))
    }

    fn room_messages<'a>(
        &'a self,
        token: &'a str,
        request: NetworkAgentRoomMessagesRequest,
    ) -> PortFuture<'a, Result<NetworkAgentRoomMessages, NetworkGatewayFailure>> {
        Box::pin(self.room_messages_internal(token, request))
    }

    fn send_message<'a>(
        &'a self,
        token: &'a str,
        draft: NetworkAgentMessageDraft,
    ) -> PortFuture<'a, Result<NetworkAgentSentMessage, NetworkGatewayFailure>> {
        Box::pin(self.send_internal(token, draft))
    }

    fn leave_and_disable<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        Box::pin(self.leave_and_disable_internal(token))
    }
}

/// 发到哪个房间：给了就必须是自己在的；没给且只在一间里就发到那间。
fn session_room(
    session: &NetworkAgentSession,
    wanted: Option<&str>,
) -> Result<MatrixRoomId, NetworkGatewayFailure> {
    let room = match wanted.map(str::trim).filter(|room| !room.is_empty()) {
        Some(wanted) => session
            .rooms
            .iter()
            .find(|room| room.matrix_room_id.as_str() == wanted)
            .ok_or(NetworkGatewayFailure::RoomNotJoined)?,
        None => match session.rooms.as_slice() {
            [only] => only,
            [] => return Err(NetworkGatewayFailure::RoomNotJoined),
            _ => return Err(NetworkGatewayFailure::RoomRequired),
        },
    };
    MatrixRoomId::new(room.matrix_room_id.as_str()).map_err(|_| NetworkGatewayFailure::Internal)
}

/// 只看、只确认一个房间时：必须是自己在的房间。没给（或者是空的）就是所有房间。
fn scoped_room(
    session: &NetworkAgentSession,
    wanted: Option<&str>,
) -> Result<Option<MatrixRoomId>, NetworkGatewayFailure> {
    match wanted.map(str::trim).filter(|room| !room.is_empty()) {
        Some(wanted) => session_room(session, Some(wanted)).map(Some),
        None => Ok(None),
    }
}

fn submission_id(value: Option<&str>) -> Result<MessageSubmissionId, NetworkGatewayFailure> {
    let Some(value) = value else {
        return Ok(MessageSubmissionId::from_uuid(Uuid::now_v7()));
    };
    Uuid::parse_str(value.trim())
        .ok()
        .filter(|uuid| uuid.get_version() == Some(Version::SortRand))
        .map(MessageSubmissionId::from_uuid)
        .ok_or(NetworkGatewayFailure::InvalidMessage("submissionId"))
}

/// 与 MCP 的聊天发言同一种形状：正文就是聊天文字，标题与摘要从正文截取。
/// 正文：公开房间里交给内容服务按服务端加密存；加密房间里先用正文密钥加密，密钥随事件由客户端加密。
fn chat_body(
    submission_id: MessageSubmissionId,
    room_id: &MatrixRoomId,
    speaker: Option<&(EncryptedSpeaker, MatrixRoomEncryption)>,
    text: &str,
) -> Result<MessageBody, NetworkGatewayFailure> {
    let media_type = ContentMediaType::new(CHAT_MEDIA_TYPE.to_owned())
        .map_err(|_| NetworkGatewayFailure::Internal)?;
    let Some((speaker, room_encryption)) = speaker else {
        return MessageBody::new(
            text.as_bytes().to_vec(),
            media_type,
            ContentEncryptionMode::ServerSide,
            None,
        )
        .map_err(|_| NetworkGatewayFailure::InvalidMessage("text"));
    };
    speaker
        .protection
        .protect(&ProtectMessageBodyRequest {
            submission_id,
            room_id,
            room_encryption: *room_encryption,
            media_type: &media_type,
            plaintext: text.as_bytes(),
            expires_at: None,
        })
        .map_err(|failure| match failure.kind() {
            ProtectMessageBodyFailureKind::InvalidBody => {
                NetworkGatewayFailure::InvalidMessage("text")
            }
            ProtectMessageBodyFailureKind::Cryptography => NetworkGatewayFailure::Internal,
        })
}

fn chat_request(
    session: &NetworkAgentSession,
    submission_id: MessageSubmissionId,
    room_id: MatrixRoomId,
    draft: NetworkAgentMessageDraft,
    body: MessageBody,
) -> Result<SendMessageRequest, NetworkGatewayFailure> {
    let summary: String = draft
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(SUMMARY_CHARACTERS)
        .collect();
    let title: String = summary.chars().take(TITLE_CHARACTERS).collect();
    let media_type = ContentMediaType::new(CHAT_MEDIA_TYPE.to_owned())
        .map_err(|_| NetworkGatewayFailure::Internal)?;
    // 先用一句占位的话单独校验提及，好告诉 Agent 错的是哪一项。
    ConversationMessage::new("·".to_owned(), draft.mentions.clone())
        .map_err(|_| NetworkGatewayFailure::InvalidMessage("mentions"))?;
    let conversation = ConversationMessage::new(draft.text.clone(), draft.mentions)
        .map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?
        .with_mentions_everyone(draft.mentions_everyone);
    let preview = MessagePreview::new(
        MessageTitle::new(title).map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?,
        MessageSummary::new(summary).map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?,
        media_type.clone(),
        None,
        MessageSensitivity::Normal,
        MessageRiskFlags::new(Vec::new()).map_err(|_| NetworkGatewayFailure::Internal)?,
    )
    .with_conversation(conversation);
    let relation = draft
        .reply_to
        .as_deref()
        .map(|target| {
            Uuid::parse_str(target.trim())
                .ok()
                .filter(|uuid| uuid.get_version() == Some(Version::SortRand))
                .map(|uuid| MessageRelation::ReplyTo(MessageId::from_uuid(uuid)))
                .ok_or(NetworkGatewayFailure::InvalidMessage("replyTo"))
        })
        .transpose()?;
    // 网络 Agent 的发言本身就是它自主决定的；没有发言授权，用网络 Agent 的 ID 占位，
    // 频率由网络 Agent 自己的限额约束。
    SendMessageRequest::new(
        submission_id,
        room_id,
        preview,
        body,
        MessageProvenance::AutonomousAgent,
        relation,
        Some(AutomationGrantId::from_uuid(
            session.network_agent_id.as_uuid(),
        )),
    )
    .map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))
}

/// @所有人只能发到私人房间（端到端加密的房间）。没进过加密房间的网络 Agent 只在公开大厅说话，
/// 没有加密客户端替它发；进过的在公开大厅说话时房间也不加密。
fn everyone_allowed(
    everyone: bool,
    speaker: Option<&(EncryptedSpeaker, MatrixRoomEncryption)>,
) -> Result<(), NetworkGatewayFailure> {
    if everyone && !matches!(speaker, Some((_, MatrixRoomEncryption::EndToEnd))) {
        return Err(NetworkGatewayFailure::InvalidMessage("mentionsEveryone"));
    }
    Ok(())
}

fn publication_failure(
    failure: MessagePublicationFailure,
    now: UtcMillis,
) -> NetworkGatewayFailure {
    match failure.kind() {
        MessagePublicationFailureKind::InvalidIntent => {
            NetworkGatewayFailure::InvalidMessage("text")
        }
        MessagePublicationFailureKind::Store
            if failure
                .store_failure()
                .is_some_and(|store| store.kind() == MessageStoreFailureKind::Conflict) =>
        {
            NetworkGatewayFailure::SubmissionConflict
        }
        MessagePublicationFailureKind::Content
            if failure.content_failure().is_some_and(|content| {
                content.kind()
                    == agent_room_bridge_core::messages::MessageContentFailureKind::Denied
            }) =>
        {
            NetworkGatewayFailure::Forbidden
        }
        MessagePublicationFailureKind::SigningUnavailable
        | MessagePublicationFailureKind::Serialization
        | MessagePublicationFailureKind::AutomationAuthorization => NetworkGatewayFailure::Internal,
        MessagePublicationFailureKind::Matrix => failure
            .matrix_failure()
            .filter(|matrix| matrix.kind() == MatrixFailureKind::RateLimited)
            .map_or(NetworkGatewayFailure::Unavailable, |matrix| {
                NetworkGatewayFailure::Agent(NetworkAgentFailure::rate_limited(matrix_retry_at(
                    matrix.retry_after(),
                    now,
                )))
            }),
        MessagePublicationFailureKind::Store | MessagePublicationFailureKind::Content => {
            NetworkGatewayFailure::Unavailable
        }
    }
}

/// Synapse 默认每人连发 10 条以后每 5 秒才放一条（`rc_message`）。被它挡下时按它给的时间让 Agent
/// 等（429 带 `Retry-After`），不说暂时不可用；它没给时间就等一秒。Matrix 没收下这条，带同一个
/// submissionId 重试照常发出。
fn matrix_retry_at(retry_after: Option<DurationMillis>, now: UtcMillis) -> UtcMillis {
    retry_after
        .or_else(|| DurationMillis::new(1_000).ok())
        .and_then(|wait| now.checked_add(wait).ok())
        .unwrap_or(now)
}

/// 收件箱这一页按等消息的规则判断：交哪些，还是等到几点再看。交的话，下标指向这一页的条目。
fn judge(
    page: &NetworkAgentInboxPage,
    rules: &WaitOptions,
    context: WakeContext<'_>,
    limit: usize,
    now_ms: i64,
    deadline_ms: i64,
) -> WaitDecision {
    let parsed: Vec<(usize, IpcMessagePreviewSummary)> = page
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            IpcMessagePreviewSummary::deserialize(&entry.preview)
                .inspect_err(|error| {
                    tracing::warn!(
                        event = entry.event_id.as_str(),
                        %error,
                        "收件箱里的预览读不出来，等消息时跳过"
                    );
                })
                .ok()
                .map(|preview| (index, preview))
        })
        .collect();
    let arrivals: Vec<Arrival<'_>> = parsed
        .iter()
        .map(|(index, preview)| Arrival {
            preview,
            arrived_at_ms: page.entries[*index].received_at.value(),
        })
        .collect();
    match decide(&arrivals, rules, context, limit, now_ms, Some(deadline_ms)) {
        WaitDecision::Deliver(mut delivery) => {
            delivery.picks = delivery.picks.iter().map(|&pick| parsed[pick].0).collect();
            WaitDecision::Deliver(delivery)
        }
        wait @ WaitDecision::Wait { .. } => wait,
    }
}

/// 只看一眼（等 0 秒、第一次取消息）：有什么给什么；只要点我的时也只给点我的。
fn peek(options: &WaitOptions) -> WaitOptions {
    WaitOptions {
        wake: if options.mentions_only {
            WakeRule::Mentions
        } else {
            WakeRule::All
        },
        mentions_only: options.mentions_only,
        ..WaitOptions::peek(None)
    }
}

fn delivered(page: NetworkAgentInboxPage, delivery: Delivery) -> NetworkAgentMessages {
    // 这一页之后还有没读到的，也算在交出去的最后一条之后。
    let unread = page
        .pending
        .saturating_sub(u64::try_from(page.entries.len()).unwrap_or(u64::MAX));
    let gaps = delivered_gaps(&page, &delivery);
    let mut previews: Vec<Option<Value>> = page
        .entries
        .into_iter()
        .map(|entry| Some(entry.preview))
        .collect();
    NetworkAgentMessages {
        // 收件箱里存的是全文；交出去时长正文只给开头，要全文按 ID 取（和本机一样）。
        messages: delivery
            .picks
            .iter()
            .filter_map(|&index| previews.get_mut(index).and_then(Option::take))
            .map(|mut preview| {
                truncate_preview_value(&mut preview);
                preview
            })
            .collect(),
        pending: page.pending,
        dropped: page.dropped,
        wake: delivery.wake,
        skipped: u64::try_from(delivery.skipped).unwrap_or(u64::MAX),
        remaining: u64::try_from(delivery.remaining)
            .unwrap_or(u64::MAX)
            .saturating_add(unread),
        gaps,
    }
}

/// 交出去的最后一条和它之前的（跳过的也算看过）前面少了的几段：补不回来的、加入之前解不开的。
/// 没确认就还在收件箱里，下次交出这几条时再说一遍。
fn delivered_gaps(page: &NetworkAgentInboxPage, delivery: &Delivery) -> Vec<IpcTimelineGap> {
    let Some(&last) = delivery.picks.iter().max() else {
        return Vec::new();
    };
    page.entries
        .iter()
        .take(last.saturating_add(1))
        .flat_map(|entry| {
            entry.gaps.iter().map(|gap| IpcTimelineGap {
                room_id: entry.room_id.as_str().to_owned(),
                after_event_id: gap
                    .after_event_id
                    .as_ref()
                    .map(|event| event.as_str().to_owned()),
                before_event_id: entry.event_id.as_str().to_owned(),
                reason: gap.reason.as_str().to_owned(),
            })
        })
        .collect()
}

/// 没有要交的，比如被新的等待顶掉。
fn nothing(page: &NetworkAgentInboxPage, reason: WakeReason) -> NetworkAgentMessages {
    NetworkAgentMessages {
        messages: Vec::new(),
        pending: page.pending,
        dropped: page.dropped,
        wake: IpcWake::empty(reason),
        skipped: 0,
        remaining: page.pending,
        gaps: Vec::new(),
    }
}

/// 从现在到某个时刻还有多久；已经过了就是零。
fn until(now_ms: i64, at_ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(at_ms.saturating_sub(now_ms)).unwrap_or(0))
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// 每个网络 Agent 同时只有一次长轮询：新来的登记后通知旧的，旧的立刻空手返回。
#[derive(Default)]
struct LongPolls {
    next: AtomicU64,
    active: Mutex<HashMap<NetworkAgentId, (u64, Arc<Notify>)>>,
}

impl LongPolls {
    fn begin(&self, agent: NetworkAgentId) -> PollTicket<'_> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let notify = Arc::new(Notify::new());
        let previous = self
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(agent, (id, notify.clone()));
        if let Some((_, previous)) = previous {
            // 旧的哪怕此刻不在等，也会在下一次等之前收到这个通知。
            previous.notify_one();
        }
        PollTicket {
            polls: self,
            agent,
            id,
            notify,
        }
    }

    /// 让这个 Agent 正在进行的长轮询立刻空手返回。
    fn supersede(&self, agent: NetworkAgentId) {
        let previous = self
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&agent);
        if let Some((_, previous)) = previous {
            previous.notify_one();
        }
    }
}

struct PollTicket<'a> {
    polls: &'a LongPolls,
    agent: NetworkAgentId,
    id: u64,
    notify: Arc<Notify>,
}

impl PollTicket<'_> {
    async fn superseded(&self) {
        self.notify.notified().await;
    }
}

impl Drop for PollTicket<'_> {
    fn drop(&mut self) {
        let mut active = self
            .polls
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if active
            .get(&self.agent)
            .is_some_and(|(current, _)| *current == self.id)
        {
            active.remove(&self.agent);
        }
    }
}
