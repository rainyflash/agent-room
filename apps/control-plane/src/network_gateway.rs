//! 网络 Agent 的网关（ADR 0010，2-收发）：服务器用 Agent 自己的 Matrix 会话替它同步房间，
//! 用本机 Bridge 同一套解析与验签把消息整理成预览，放进它的收件箱。Agent 凭令牌长轮询取，
//! 只有显式确认才往前走。一个 Agent 同时只有一次长轮询，新来的会让旧的立刻空手返回。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use agent_room_application::{
    content::ContentUseCases,
    network_agents::{NetworkAgentFailure, NetworkAgentSession, NetworkAgentUseCases},
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock, MatrixEventId,
        MatrixRoomId, MatrixSyncBatch, NetworkAgentAckOutcome, NetworkAgentInboxAppend,
        NetworkAgentInboxChange, NetworkAgentInboxPage, NetworkAgentInboxStore,
        NetworkAgentMatrixGateway, NetworkAgentSubmissionStore, NetworkAgentSyncRequest,
        PortFuture,
    },
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    agent_verification::{
        AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
    },
    messages::{
        MessageBody, MessagePublicationDependencies, MessagePublicationFailure,
        MessagePublicationFailureKind, MessagePublicationOutcome, MessagePublicationService,
        MessageStoreFailureKind, MessageSyncDependencies, MessageSyncService, SendMessageRequest,
    },
};
use agent_room_domain::{
    content::{ContentEncryptionMode, ContentMediaType},
    ids::{AutomationGrantId, MessageId, MessageSubmissionId, NetworkAgentId},
    messages::{
        ConversationMessage, MessagePreview, MessageProvenance, MessageRelation, MessageRiskFlags,
        MessageSensitivity, MessageSummary, MessageTitle,
    },
};
use serde_json::Value;
use tokio::{sync::Notify, time::Instant};
use uuid::{Uuid, Version};

mod projection;
mod speaking;
#[cfg(test)]
mod tests;

/// 长轮询最多等这么久。
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(30);
/// 一次最多取这么多条。
pub(crate) const MAX_PAGE: u16 = 50;
/// 第一次同步只带每个房间最近几条，给 Agent 一点上下文。
const FIRST_SYNC_TIMELINE_LIMIT: u16 = 20;
const SYNC_TIMELINE_LIMIT: u16 = 50;
/// 最多留这么多条没确认的；再多就丢掉最早的，并在下次取消息时告诉 Agent 丢了几条。
const INBOX_CAPACITY: u32 = 200;
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
    /// 提及的 Matrix 用户，最多 8 个。
    pub(crate) mentions: Vec<String>,
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
}

/// 网络 Agent 收消息的接口；HTTP 层只认这个，便于单独测试。
pub(crate) trait NetworkAgentMessaging: Send + Sync {
    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        wait: Duration,
        limit: u16,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>>;

    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>>;

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

pub(crate) struct NetworkGatewayDependencies {
    pub(crate) agents: Arc<dyn NetworkAgentUseCases>,
    pub(crate) inbox: Arc<dyn NetworkAgentInboxStore>,
    pub(crate) submissions: Arc<dyn NetworkAgentSubmissionStore>,
    pub(crate) matrix: Arc<dyn NetworkAgentMatrixGateway>,
    pub(crate) content: Arc<dyn ContentUseCases>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) clock: Arc<dyn Clock>,
}

pub(crate) struct NetworkGateway {
    agents: Arc<dyn NetworkAgentUseCases>,
    inbox: Arc<dyn NetworkAgentInboxStore>,
    submissions: Arc<dyn NetworkAgentSubmissionStore>,
    matrix: Arc<dyn NetworkAgentMatrixGateway>,
    content: Arc<dyn ContentUseCases>,
    verification: Arc<dyn AgentInstanceVerificationRepository>,
    signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    clock: Arc<dyn Clock>,
    polls: LongPolls,
}

impl NetworkGateway {
    pub(crate) fn new(dependencies: NetworkGatewayDependencies) -> Self {
        Self {
            agents: dependencies.agents,
            inbox: dependencies.inbox,
            submissions: dependencies.submissions,
            matrix: dependencies.matrix,
            content: dependencies.content,
            verification: dependencies.verification,
            signatures: dependencies.signatures,
            clock: dependencies.clock,
            polls: LongPolls::default(),
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
        let request = chat_request(&session, submission_id, room.clone(), draft)?;
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
        let publication = MessagePublicationService::new(MessagePublicationDependencies {
            identity,
            signer: Arc::new(signer),
            publisher: Arc::new(speaking::SessionPublisher {
                matrix: self.matrix.clone(),
                access_token: session.matrix_access_token.clone(),
            }),
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
            Err(failure) => return Err(publication_failure(failure)),
        };
        Ok(NetworkAgentSentMessage {
            submission: submission_id,
            room: room.as_str().to_owned(),
            event: event_id,
        })
    }

    /// 尽量离开所有房间，再作废令牌。离开失败不挡作废：令牌作废才是停用的关键。
    async fn leave_and_disable_internal(&self, token: &str) -> Result<(), NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
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
                    "网络 Agent 停用时没能离开房间，令牌照样作废"
                );
            }
        }
        self.agents
            .disable(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)
    }

    fn submissions_for(&self, session: &NetworkAgentSession) -> speaking::AgentSubmissions {
        speaking::AgentSubmissions {
            store: self.submissions.clone(),
            id: session.network_agent_id,
            clock: self.clock.clone(),
        }
    }

    /// 取还没确认的消息；没有时最多等 `wait`，期间一有新消息就返回。
    async fn wait_internal(
        &self,
        token: &str,
        wait: Duration,
        limit: u16,
    ) -> Result<NetworkAgentMessages, NetworkGatewayFailure> {
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let poll = self.polls.begin(session.network_agent_id);
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        let limit = limit.clamp(1, MAX_PAGE);
        loop {
            let page = self
                .inbox
                .pending(session.network_agent_id, limit)
                .await
                .map_err(|_| NetworkGatewayFailure::Unavailable)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            // 第一次同步不等：先把房间里最近的几条拿来当上下文。
            let first = page.sync_token.is_none();
            if !page.entries.is_empty() || (remaining.is_zero() && !first) {
                return Ok(messages(page));
            }
            let request = NetworkAgentSyncRequest {
                since: page.sync_token.clone(),
                timeout_millis: if first {
                    0
                } else {
                    u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX)
                },
                timeline_limit: if first {
                    FIRST_SYNC_TIMELINE_LIMIT
                } else {
                    SYNC_TIMELINE_LIMIT
                },
            };
            let batch = tokio::select! {
                result = self.matrix.sync(&session.matrix_access_token, &request) => {
                    result.map_err(|_| NetworkGatewayFailure::Unavailable)?
                }
                () = poll.superseded() => return Ok(messages(page)),
            };
            let changes = self.changes(&session, &batch).await?;
            // 位置对不上说明另一次同步已经写过（例如被取代的旧长轮询），下一轮从新位置接着来。
            self.inbox
                .append(&NetworkAgentInboxAppend {
                    id: session.network_agent_id,
                    expected_sync_token: page.sync_token,
                    next_sync_token: batch.next_batch().clone(),
                    changes,
                    received_at: self.clock.now(),
                    capacity: INBOX_CAPACITY,
                })
                .await
                .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        }
    }

    /// 确认处理到这一条（含）为止。
    async fn acknowledge_internal(
        &self,
        token: &str,
        event_id: &str,
    ) -> Result<NetworkAgentAckOutcome, NetworkGatewayFailure> {
        let event_id =
            MatrixEventId::new(event_id).map_err(|_| NetworkGatewayFailure::InvalidEvent)?;
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        self.inbox
            .acknowledge(session.network_agent_id, &event_id)
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
        Ok(projection::inbox_changes(captured.take(), session.agent_id))
    }
}

impl NetworkAgentMessaging for NetworkGateway {
    fn wait_for_messages<'a>(
        &'a self,
        token: &'a str,
        wait: Duration,
        limit: u16,
    ) -> PortFuture<'a, Result<NetworkAgentMessages, NetworkGatewayFailure>> {
        Box::pin(self.wait_internal(token, wait, limit))
    }

    fn acknowledge<'a>(
        &'a self,
        token: &'a str,
        event_id: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentAckOutcome, NetworkGatewayFailure>> {
        Box::pin(self.acknowledge_internal(token, event_id))
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
fn chat_request(
    session: &NetworkAgentSession,
    submission_id: MessageSubmissionId,
    room_id: MatrixRoomId,
    draft: NetworkAgentMessageDraft,
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
        .map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?;
    let preview = MessagePreview::new(
        MessageTitle::new(title).map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?,
        MessageSummary::new(summary).map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?,
        media_type.clone(),
        None,
        MessageSensitivity::Normal,
        MessageRiskFlags::new(Vec::new()).map_err(|_| NetworkGatewayFailure::Internal)?,
    )
    .with_conversation(conversation);
    let body = MessageBody::new(
        draft.text.into_bytes(),
        media_type,
        ContentEncryptionMode::ServerSide,
        None,
    )
    .map_err(|_| NetworkGatewayFailure::InvalidMessage("text"))?;
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

fn publication_failure(failure: MessagePublicationFailure) -> NetworkGatewayFailure {
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
        MessagePublicationFailureKind::Store
        | MessagePublicationFailureKind::Content
        | MessagePublicationFailureKind::Matrix => NetworkGatewayFailure::Unavailable,
    }
}

fn messages(page: NetworkAgentInboxPage) -> NetworkAgentMessages {
    NetworkAgentMessages {
        messages: page
            .entries
            .into_iter()
            .map(|entry| entry.preview)
            .collect(),
        pending: page.pending,
        dropped: page.dropped,
    }
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
