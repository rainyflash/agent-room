use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    content::{
        BeginContentUploadOutcome, BeginContentUploadRequest, BeginContentUploadResult,
        BindContentEventOutcome, BindContentEventRequest, BindContentEventResult,
        CompleteContentUploadOutcome, CompleteContentUploadRequest, CompleteContentUploadResult,
        ContentUseCases, IssueContentReadTicketRequest, IssueContentReadTicketResult,
        IssuedContentReadTicket, OpenContentRequest, OpenContentResult, OpenedVerifiedContent,
        RedactContentOutcome, RedactContentRequest, RedactContentResult,
    },
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentAdmission,
        NetworkAgentEncryptionSecrets, NetworkAgentFailure, NetworkAgentFailureKind,
        NetworkAgentKnock, NetworkAgentKnockStatus, NetworkAgentLobby, NetworkAgentMatrixDevice,
        NetworkAgentPendingExit, NetworkAgentPlacement, NetworkAgentResult, NetworkAgentRoom,
        NetworkAgentRoomRequest, NetworkAgentSession, NetworkAgentTarget, NetworkAgentUseCases,
        NetworkAgentView,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRecord,
        AgentInstanceVerificationRepository, Clock, ContentAccessMode, ContentAccessPolicy,
        DeviceSignature, MatrixAcceptedEvent, MatrixBackfillPage, MatrixBackfillRequest,
        MatrixBackfillToken, MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventId,
        MatrixEventType, MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixOperation,
        MatrixPowerLevel, MatrixReceipt, MatrixResult, MatrixRoomAliasLocalpart,
        MatrixRoomAuthority, MatrixRoomAuthorityGateway, MatrixRoomEncryption, MatrixRoomId,
        MatrixRoomSync, MatrixRoomSyncKind, MatrixSessionMetadata, MatrixStateEvent,
        MatrixStateKey, MatrixSyncBatch, MatrixSyncRequest, MatrixSyncToken, MatrixTimelineEvent,
        MatrixTransactionId, MatrixUserId, NetworkAgentAckOutcome, NetworkAgentBeforeJoinGap,
        NetworkAgentGapReason, NetworkAgentHistoryDirection, NetworkAgentHistoryFilter,
        NetworkAgentHistorySender, NetworkAgentInboxAppend, NetworkAgentInboxAppendOutcome,
        NetworkAgentInboxChange, NetworkAgentInboxEntry, NetworkAgentInboxPage,
        NetworkAgentInboxStore, NetworkAgentMatrixGateway, NetworkAgentMessageActor,
        NetworkAgentMessageHistory, NetworkAgentMessageRef, NetworkAgentMessageRetention,
        NetworkAgentRoomRecord, NetworkAgentStoredMessage, NetworkAgentSubmissionClaim,
        NetworkAgentSubmissionClaimOutcome, NetworkAgentSubmissionRecord,
        NetworkAgentSubmissionState, NetworkAgentSubmissionStore, NetworkAgentSyncRequest,
        NetworkAgentTimelineGap, PortFuture, SecretValue,
    },
};
use agent_room_bridge_core::{
    matrix_recovery::{MatrixRecoveryCommand, MatrixRecoveryResult},
    matrix_security::{
        MatrixSecurityCommand, MatrixSecurityFailure, MatrixSecurityGateway, MatrixSecurityResult,
    },
    messages::MessageBodyProtectionService,
    status::WAIT_IDLE_TIMEOUT,
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState,
    agents::AgentInstancePublicSigningKey,
    content::{
        ContentEncryptionMode, ContentLifecycleState, ContentObject, ContentObjectFields,
        ContentScanState, ContentStorageKey,
    },
    ids::{
        AgentId, AgentInstanceId, MessageId, MessageSubmissionId, NetworkAgentId, PrincipalId,
        RoomCatalogId,
    },
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use agent_room_identity_adapter::Ed25519DeviceSigningKey;
use agent_room_message_crypto_adapter::{AesGcmMessageContentCipher, MessageContentRootKey};
use serde_json::{Value, json};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{
    AdmittedAgentEntry, EncryptedSessions, EncryptedSpeaker, NetworkAgentCleanupOutcome,
    NetworkAgentEntry, NetworkAgentMessageDraft, NetworkAgentMessaging, NetworkAgentWait,
    NetworkGateway, NetworkGatewayDependencies, NetworkGatewayFailure,
};
use agent_room_bridge_ipc::wake::{WaitOptions, WakeReason, WakeRule};

const TOKEN: &str = "network-agent-token";
const ROOM: &str = "!lobby:matrix.test";
const SECOND_ROOM: &str = "!second:matrix.test";
const PRIVATE_ROOM: &str = "!private:matrix.test";
const PRINCIPAL: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e53";
const NETWORK_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
const OWN_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";
const OWN_INSTANCE: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e52";
const OTHER_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e61";
const OTHER_INSTANCE: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e62";
/// 验签替身把全是 0xFF 的签名当成伪造的。
const FORGED_SIGNATURE: [u8; 64] = [0xFF; 64];

// ---------- 假实现 ----------

/// 网关看到的网络 Agent：令牌对上就给会话，发言额度可以设成已用完，停用的令牌记下来。
struct FakeAgents {
    seed: SecretValue,
    rooms: Vec<NetworkAgentRoomRecord>,
    quota: Mutex<Option<NetworkAgentFailure>>,
    quota_taken: Mutex<u32>,
    disabled: Mutex<Vec<String>>,
    /// 定时清理：停用了几个闲置的、待离开的有哪些、记为已离开的有哪些、删了钥匙的有哪些。
    /// 记为已离开、还没删钥匙的，就等着删钥匙。
    stale: Mutex<usize>,
    exits: Mutex<Vec<NetworkAgentPendingExit>>,
    rooms_left: Mutex<Vec<NetworkAgentId>>,
    keys_deleted: Mutex<Vec<NetworkAgentId>>,
    /// 进过加密房间的时刻；有值时网关改用加密客户端同步。记入库（`mark_encrypted`）不改它：
    /// 切过去之前取的会话里本来就没有，网关得靠自己记着。
    encrypted_since: Mutex<Option<UtcMillis>>,
    /// 按顺序记下创建、放行、记入库、准备加密客户端、进房间这几步，看先后。
    log: Arc<Mutex<Vec<String>>>,
    /// 放行的结果，按顺序给出。
    admissions: Mutex<VecDeque<NetworkAgentResult<NetworkAgentAdmission>>>,
    entered: Mutex<Vec<NetworkAgentTarget>>,
}

impl FakeAgents {
    fn in_rooms(rooms: &[&str]) -> Self {
        Self {
            seed: Ed25519DeviceSigningKey::generate()
                .unwrap()
                .encoded_seed()
                .unwrap(),
            rooms: rooms
                .iter()
                .map(|room| NetworkAgentRoomRecord {
                    catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
                    matrix_room_id: MatrixRoomReference::new((*room).to_owned()).unwrap(),
                    joined_at: UtcMillis::new(1).unwrap(),
                    name: None,
                })
                .collect(),
            quota: Mutex::new(None),
            quota_taken: Mutex::new(0),
            disabled: Mutex::new(Vec::new()),
            stale: Mutex::new(0),
            exits: Mutex::new(Vec::new()),
            rooms_left: Mutex::new(Vec::new()),
            keys_deleted: Mutex::new(Vec::new()),
            encrypted_since: Mutex::new(None),
            log: Arc::new(Mutex::new(Vec::new())),
            admissions: Mutex::new(VecDeque::new()),
            entered: Mutex::new(Vec::new()),
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn own_session(&self) -> NetworkAgentSession {
        NetworkAgentSession {
            network_agent_id: network_agent_id(),
            principal_id: PrincipalId::from_uuid(uuid(PRINCIPAL)),
            agent_id: agent(OWN_AGENT),
            agent_instance_id: instance(OWN_INSTANCE),
            display_name: "Scout".to_owned(),
            agent_matrix_user_id: matrix_user(OWN_AGENT),
            matrix_access_token: SecretValue::new("syt_scout").unwrap(),
            instance_signing_seed: self.seed.clone(),
            rooms: self.rooms.clone(),
            matrix_device_id: format!("AR_{}", uuid(OWN_INSTANCE).simple()),
            encrypted_since: *self.encrypted_since.lock().unwrap(),
        }
    }
}

impl NetworkAgentUseCases for FakeAgents {
    /// 凭口令创建：只放行了私人房间，还没进。拿房间号创建：只敲了门。
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
        self.log.lock().unwrap().push("create".to_owned());
        let placement = match request.room {
            NetworkAgentRoomRequest::Code(_) => NetworkAgentPlacement::Admitted(private_room()),
            NetworkAgentRoomRequest::Lobby(Some(_)) => NetworkAgentPlacement::Knocked(knock()),
            NetworkAgentRoomRequest::Lobby(None) => unreachable!("网关测试只建私人房间的"),
        };
        Box::pin(async move {
            Ok(CreatedNetworkAgent {
                network_agent_id: network_agent_id(),
                agent_id: agent(OWN_AGENT),
                display_name: request.name,
                token: SecretValue::new(TOKEN).unwrap(),
                placement,
            })
        })
    }

    fn me<'a>(&'a self, _token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>> {
        unreachable!("网关不查看自己")
    }

    fn disable<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>> {
        self.disabled.lock().unwrap().push(token.to_owned());
        Box::pin(async { Ok(()) })
    }

    fn session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
        let result = if token == TOKEN {
            Ok(self.own_session())
        } else {
            Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            ))
        };
        Box::pin(async move { result })
    }

    /// 按 Agent 取会话：只认自己，停用过的不认。
    fn session_of_agent(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentSession>> {
        self.log.lock().unwrap().push("session_of_agent".to_owned());
        let result = if agent_id == agent(OWN_AGENT) && self.disabled.lock().unwrap().is_empty() {
            Ok(self.own_session())
        } else {
            Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            ))
        };
        Box::pin(async move { result })
    }

    fn enter_admitted(
        &self,
        _agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentRoom>> {
        self.log.lock().unwrap().push("enter_admitted".to_owned());
        self.entered
            .lock()
            .unwrap()
            .push(NetworkAgentTarget::Private(room.clone()));
        Box::pin(async move { Ok(room) })
    }

    fn take_message_quota(&self, _id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        let result = if let Some(failure) = self.quota.lock().unwrap().clone() {
            Err(failure)
        } else {
            *self.quota_taken.lock().unwrap() += 1;
            Ok(())
        };
        Box::pin(async move { result })
    }

    fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>> {
        let stale = std::mem::take(&mut *self.stale.lock().unwrap());
        Box::pin(async move { Ok(stale) })
    }

    fn pending_exits(
        &self,
        _limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>> {
        let left = self.rooms_left.lock().unwrap().clone();
        let exits = self
            .exits
            .lock()
            .unwrap()
            .iter()
            .filter(|exit| {
                let id = match exit {
                    NetworkAgentPendingExit::Session(session) => session.network_agent_id,
                    NetworkAgentPendingExit::Unopenable(id) => *id,
                };
                !left.contains(&id)
            })
            .cloned()
            .collect();
        Box::pin(async move { Ok(exits) })
    }

    fn mark_rooms_left(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        self.rooms_left.lock().unwrap().push(id);
        Box::pin(async { Ok(()) })
    }

    fn pending_key_deletions(
        &self,
        _limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentId>>> {
        let deleted = self.keys_deleted.lock().unwrap().clone();
        let pending = self
            .rooms_left
            .lock()
            .unwrap()
            .iter()
            .filter(|id| !deleted.contains(id))
            .copied()
            .collect();
        Box::pin(async move { Ok(pending) })
    }

    fn delete_keys(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        self.log.lock().unwrap().push("delete_keys".to_owned());
        self.keys_deleted.lock().unwrap().push(id);
        Box::pin(async { Ok(()) })
    }

    fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>> {
        unreachable!("网关不列大厅")
    }

    fn admit<'a>(
        &'a self,
        _token: &'a str,
        room: NetworkAgentRoomRequest,
        _source_digest: [u8; 32],
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentAdmission>> {
        self.log.lock().unwrap().push(format!("admit {room:?}"));
        let admission = self
            .admissions
            .lock()
            .unwrap()
            .pop_front()
            .expect("测试给了放行结果");
        Box::pin(async move { admission })
    }

    fn enter<'a>(
        &'a self,
        _token: &'a str,
        target: NetworkAgentTarget,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoom>> {
        self.log.lock().unwrap().push("enter".to_owned());
        self.entered.lock().unwrap().push(target.clone());
        let room = match target {
            NetworkAgentTarget::Private(room) => room,
            NetworkAgentTarget::Lobby(catalog_id) => NetworkAgentRoom {
                catalog_id,
                matrix_room_id: MatrixRoomReference::new(SECOND_ROOM.to_owned()).unwrap(),
                name: "Rust 夜谈".to_owned(),
            },
        };
        Box::pin(async move { Ok(room) })
    }

    fn mark_encrypted(&self, _id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<UtcMillis>> {
        self.log.lock().unwrap().push("mark_encrypted".to_owned());
        Box::pin(async { Ok(UtcMillis::new(42).unwrap()) })
    }

    fn encryption_secrets(
        &self,
        _id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentEncryptionSecrets>> {
        unreachable!("网关测试里的加密客户端是替身")
    }

    fn store_recovery_credential<'a>(
        &'a self,
        _id: NetworkAgentId,
        _credential: &'a SecretValue,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        unreachable!("网关测试里的加密客户端是替身")
    }
    fn replace_matrix_device(
        &self,
        _id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentMatrixDevice>> {
        unreachable!("网关测试里的加密客户端是替身")
    }
}

/// 与 Postgres 实现同样语义的提交记录。
#[derive(Default)]
struct MemorySubmissions {
    records: Mutex<HashMap<MessageSubmissionId, NetworkAgentSubmissionRecord>>,
}

impl MemorySubmissions {
    fn state(&self, submission_id: MessageSubmissionId) -> Option<NetworkAgentSubmissionState> {
        self.records
            .lock()
            .unwrap()
            .get(&submission_id)
            .map(|record| record.state)
    }
}

fn conflict() -> RepositoryError {
    RepositoryError::new("test.submission", RepositoryErrorKind::Conflict)
}

impl NetworkAgentSubmissionStore for MemorySubmissions {
    fn claim<'a>(
        &'a self,
        _id: NetworkAgentId,
        claim: &'a NetworkAgentSubmissionClaim,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentSubmissionClaimOutcome>> {
        let mut records = self.records.lock().unwrap();
        let outcome = match records.get(&claim.submission_id) {
            Some(existing)
                if existing.kind == claim.kind
                    && existing.fingerprint == claim.fingerprint
                    && existing.transaction_id == claim.transaction_id =>
            {
                Ok(NetworkAgentSubmissionClaimOutcome::Existing(
                    existing.clone(),
                ))
            }
            Some(_) => Err(conflict()),
            None => {
                let record = NetworkAgentSubmissionRecord {
                    submission_id: claim.submission_id,
                    kind: claim.kind,
                    fingerprint: claim.fingerprint,
                    transaction_id: claim.transaction_id.clone(),
                    state: NetworkAgentSubmissionState::Claimed,
                    event_id: None,
                };
                records.insert(claim.submission_id, record.clone());
                Ok(NetworkAgentSubmissionClaimOutcome::Created(record))
            }
        };
        Box::pin(async move { outcome })
    }

    fn mark_submit_unknown(
        &self,
        _id: NetworkAgentId,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentSubmissionRecord>> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(&submission_id).unwrap();
        if record.state == NetworkAgentSubmissionState::Claimed {
            record.state = NetworkAgentSubmissionState::SubmitUnknown;
        }
        let record = record.clone();
        Box::pin(async move { Ok(record) })
    }

    fn mark_accepted<'a>(
        &'a self,
        _id: NetworkAgentId,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentSubmissionRecord>> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(&submission_id).unwrap();
        let result = if record
            .event_id
            .as_ref()
            .is_some_and(|existing| existing != event_id)
        {
            Err(conflict())
        } else {
            if record.state != NetworkAgentSubmissionState::Bound {
                record.state = NetworkAgentSubmissionState::Accepted;
                record.event_id = Some(event_id.clone());
            }
            Ok(record.clone())
        };
        Box::pin(async move { result })
    }

    fn mark_bound(
        &self,
        _id: NetworkAgentId,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentSubmissionRecord>> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(&submission_id).unwrap();
        record.state = NetworkAgentSubmissionState::Bound;
        let record = record.clone();
        Box::pin(async move { Ok(record) })
    }

    fn observe_transaction<'a>(
        &'a self,
        _id: NetworkAgentId,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<Option<NetworkAgentSubmissionRecord>>> {
        let mut records = self.records.lock().unwrap();
        let found = records
            .values_mut()
            .find(|record| &record.transaction_id == transaction_id)
            .map(|record| {
                if record.state != NetworkAgentSubmissionState::Bound {
                    record.state = NetworkAgentSubmissionState::Accepted;
                    record.event_id = Some(event_id.clone());
                }
                record.clone()
            });
        Box::pin(async move { Ok(found) })
    }
}

/// 进程内内容服务的替身：记下谁以什么身份上传了什么、绑到了哪个事件。
#[derive(Default)]
struct FakeContent {
    uploads: Mutex<Vec<(BeginContentUploadRequest, Vec<u8>)>>,
    bindings: Mutex<Vec<BindContentEventRequest>>,
}

fn content_object(request: &BeginContentUploadRequest) -> ContentObject {
    let id = agent_room_domain::ids::ContentId::from_uuid(request.request_id.as_uuid());
    ContentObject::begin_upload(ContentObjectFields {
        id,
        owner_principal_id: request.owner_principal_id,
        storage_key: ContentStorageKey::new(format!("content/{id}/opaque-random-suffix")).unwrap(),
        digest: request.digest,
        byte_length: request.byte_length,
        media_type: request.media_type.clone(),
        encryption_mode: request.encryption_mode,
        // 客户端密文服务端看不到，不扫描。
        scan_state: if request.encryption_mode == ContentEncryptionMode::ClientE2ee {
            ContentScanState::NotApplicable
        } else {
            ContentScanState::Clean
        },
        lifecycle_state: ContentLifecycleState::Uploading,
        expires_at: None,
        retention_expires_at: None,
        created_at: UtcMillis::new(1).unwrap(),
        deleted_at: None,
    })
    .unwrap()
}

impl ContentUseCases for FakeContent {
    fn begin_upload(
        &self,
        request: BeginContentUploadRequest,
    ) -> PortFuture<'_, BeginContentUploadResult<BeginContentUploadOutcome>> {
        let content = content_object(&request);
        let policy = ContentAccessPolicy::new(
            content.id(),
            request.matrix_room_id.clone(),
            request.access_mode,
            UtcMillis::new(1).unwrap(),
        );
        self.uploads.lock().unwrap().push((request, Vec::new()));
        Box::pin(async move {
            Ok(BeginContentUploadOutcome::Created {
                content,
                access_policy: policy,
            })
        })
    }

    fn complete_upload(
        &self,
        request: CompleteContentUploadRequest,
    ) -> PortFuture<'_, CompleteContentUploadResult<CompleteContentUploadOutcome>> {
        Box::pin(async move {
            use futures_util::StreamExt as _;
            let mut body = Vec::new();
            let mut stream = request.body;
            while let Some(chunk) = stream.next().await {
                body.extend_from_slice(&chunk.unwrap());
            }
            let mut uploads = self.uploads.lock().unwrap();
            let (begun, bytes) = uploads.last_mut().unwrap();
            *bytes = body;
            let mut content = content_object(begun);
            content.activate().unwrap();
            Ok(CompleteContentUploadOutcome::Activated(content))
        })
    }

    fn bind_event(
        &self,
        request: BindContentEventRequest,
    ) -> PortFuture<'_, BindContentEventResult<BindContentEventOutcome>> {
        let policy = ContentAccessPolicy::new(
            request.content_id,
            request.matrix_room_id.clone(),
            ContentAccessMode::RoomMember,
            UtcMillis::new(1).unwrap(),
        );
        self.bindings.lock().unwrap().push(request);
        Box::pin(async move { Ok(BindContentEventOutcome::Bound(policy)) })
    }

    fn redact(
        &self,
        _request: RedactContentRequest,
    ) -> PortFuture<'_, RedactContentResult<RedactContentOutcome>> {
        unreachable!("网络 Agent 这一步不撤回")
    }

    fn issue_read_ticket(
        &self,
        _request: IssueContentReadTicketRequest,
    ) -> PortFuture<'_, IssueContentReadTicketResult<IssuedContentReadTicket>> {
        unreachable!("网关不签读取票据")
    }

    fn open(
        &self,
        _request: OpenContentRequest,
    ) -> PortFuture<'_, OpenContentResult<OpenedVerifiedContent>> {
        unreachable!("网关不读正文")
    }
}

/// 与 Postgres 实现同样语义的收件箱：按到达编号，确认到哪条就删到哪条；可以只读、只确认
/// 一个房间的，每个房间各自限额。
#[derive(Default)]
struct MemoryInbox {
    state: Mutex<InboxState>,
    /// 删过了保留期的副本时问的时间和留多久；按什么删由 Postgres 的测试管。
    pruned: Mutex<Vec<(UtcMillis, NetworkAgentMessageRetention)>>,
}

#[derive(Default)]
struct InboxState {
    sync_token: Option<MatrixSyncToken>,
    sequence: u64,
    dropped: u64,
    entries: Vec<InboxRow>,
    /// 消息记录：确认过的、自己发的也在，每个房间各自限额。
    history: Vec<HistoryRow>,
    /// 加入之前解不开的一段：哪一次加入（服务器收到加入的时间），说过没有。
    before_join: HashMap<MatrixRoomId, (UtcMillis, bool)>,
}

struct HistoryRow {
    stored: NetworkAgentStoredMessage,
    actor_key: String,
    actor: NetworkAgentMessageActor,
    mentions_me: bool,
}

impl HistoryRow {
    fn wanted(&self, filter: &NetworkAgentHistoryFilter) -> bool {
        (!filter.mentions_me || self.mentions_me)
            && match &filter.from {
                None => true,
                Some(NetworkAgentHistorySender::MatrixUserId(user)) => {
                    self.actor.matrix_user_id == *user
                }
                Some(NetworkAgentHistorySender::NameFolded(name)) => {
                    self.actor.name_folded == *name
                }
            }
    }
}

struct InboxRow {
    sequence: u64,
    event_id: MatrixEventId,
    room_id: MatrixRoomId,
    message_id: MessageId,
    actor_key: String,
    preview: Value,
    received_at: UtcMillis,
    sent_at: UtcMillis,
    gap: Option<NetworkAgentTimelineGap>,
    before_join: bool,
}

impl InboxRow {
    fn in_scope(&self, room: Option<&MatrixRoomId>) -> bool {
        room.is_none_or(|room| *room == self.room_id)
    }

    /// 和数据库读出来的一样：加入之前解不开的在前，补不回来的在后。
    fn gaps(&self) -> Vec<NetworkAgentTimelineGap> {
        let before_join = self.before_join.then_some(NetworkAgentTimelineGap {
            after_event_id: None,
            reason: NetworkAgentGapReason::UndecryptableBeforeJoin,
        });
        before_join.into_iter().chain(self.gap.clone()).collect()
    }
}

impl InboxState {
    fn pending(&self, room: Option<&MatrixRoomId>) -> u64 {
        u64::try_from(self.entries.iter().filter(|row| row.in_scope(room)).count()).unwrap()
    }

    /// 和数据库一样：同一次加入不再记，更晚的一次重新记。
    fn owe_before_join(&mut self, gaps: &[NetworkAgentBeforeJoinGap]) {
        for gap in gaps {
            let newer = self
                .before_join
                .get(&gap.room_id)
                .is_none_or(|(joined_at, _)| *joined_at < gap.joined_at);
            if newer {
                self.before_join
                    .insert(gap.room_id.clone(), (gap.joined_at, false));
            }
        }
    }

    /// 欠着加入之前那一段的房间，挂到第一条进收件箱的消息上，这次加入只说一次。
    fn tell_before_join(&mut self, room: &MatrixRoomId) -> bool {
        match self.before_join.get_mut(room) {
            Some((_, told)) if !*told => {
                *told = true;
                true
            }
            _ => false,
        }
    }

    /// 消息记录里每个房间只留最近的几条。
    fn trim_history(&mut self, capacity: u32) {
        let capacity = usize::try_from(capacity).unwrap();
        let mut kept: HashMap<MatrixRoomId, usize> = HashMap::new();
        for index in (0..self.history.len()).rev() {
            let count = kept
                .entry(self.history[index].stored.room_id.clone())
                .or_default();
            *count += 1;
            if *count > capacity {
                self.history.remove(index);
            }
        }
    }

    /// 一个房间里超过上限时丢掉这个房间最早的。
    fn trim(&mut self, capacity: u32) {
        let capacity = usize::try_from(capacity).unwrap();
        let mut kept: HashMap<MatrixRoomId, usize> = HashMap::new();
        let mut dropped = 0;
        for index in (0..self.entries.len()).rev() {
            let count = kept.entry(self.entries[index].room_id.clone()).or_default();
            *count += 1;
            if *count > capacity {
                self.entries.remove(index);
                dropped += 1;
            }
        }
        self.dropped += dropped;
    }
}

impl MemoryInbox {
    fn sync_token(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .sync_token
            .as_ref()
            .map(|token| token.as_str().to_owned())
    }
}

impl NetworkAgentInboxStore for MemoryInbox {
    fn pending<'a>(
        &'a self,
        _id: NetworkAgentId,
        room: Option<&'a MatrixRoomId>,
        limit: u16,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentInboxPage>> {
        let state = self.state.lock().unwrap();
        let page = NetworkAgentInboxPage {
            sync_token: state.sync_token.clone(),
            entries: state
                .entries
                .iter()
                .filter(|row| row.in_scope(room))
                .take(usize::from(limit))
                .map(|row| NetworkAgentInboxEntry {
                    sequence: row.sequence,
                    event_id: row.event_id.clone(),
                    room_id: row.room_id.clone(),
                    preview: row.preview.clone(),
                    received_at: row.received_at,
                    gaps: row.gaps(),
                })
                .collect(),
            pending: state.pending(room),
            dropped: state.dropped,
        };
        Box::pin(async move { Ok(page) })
    }

    fn append<'a>(
        &'a self,
        append: &'a NetworkAgentInboxAppend,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentInboxAppendOutcome>> {
        let mut state = self.state.lock().unwrap();
        let outcome = if state.sync_token == append.expected_sync_token {
            let mut appended = 0;
            state.owe_before_join(&append.undecryptable_before_join);
            for change in &append.changes {
                match change {
                    NetworkAgentInboxChange::Message(message) => {
                        // 和数据库一样先记进消息记录：记过的是重复的，收件箱也不再写。
                        if state
                            .history
                            .iter()
                            .any(|row| row.stored.event_id == message.event_id)
                        {
                            continue;
                        }
                        state.sequence += 1;
                        let sequence = state.sequence;
                        state.history.push(HistoryRow {
                            stored: NetworkAgentStoredMessage {
                                sequence,
                                event_id: message.event_id.clone(),
                                room_id: message.room_id.clone(),
                                message_id: message.message_id,
                                preview: message.preview.clone(),
                            },
                            actor_key: message.actor_key.clone(),
                            actor: message.actor.clone(),
                            mentions_me: message.mentions_me,
                        });
                        if message.from_me {
                            continue;
                        }
                        let before_join = state.tell_before_join(&message.room_id);
                        state.entries.push(InboxRow {
                            sequence,
                            event_id: message.event_id.clone(),
                            room_id: message.room_id.clone(),
                            message_id: message.message_id,
                            actor_key: message.actor_key.clone(),
                            preview: message.preview.clone(),
                            received_at: append.received_at,
                            sent_at: message.sent_at,
                            gap: message.gap.clone(),
                            before_join,
                        });
                        appended += 1;
                    }
                    NetworkAgentInboxChange::Replace {
                        message_id,
                        actor_key,
                        patch,
                        ..
                    } => {
                        for row in &mut state.entries {
                            if row.message_id == *message_id && row.actor_key == *actor_key {
                                for (key, value) in patch.as_object().unwrap() {
                                    row.preview[key] = value.clone();
                                }
                            }
                        }
                        for row in &mut state.history {
                            if row.stored.message_id == *message_id && row.actor_key == *actor_key {
                                for (key, value) in patch.as_object().unwrap() {
                                    row.stored.preview[key] = value.clone();
                                }
                            }
                        }
                    }
                    NetworkAgentInboxChange::Redact {
                        message_id,
                        actor_key,
                        ..
                    } => {
                        state.entries.retain(|row| {
                            !(row.message_id == *message_id && row.actor_key == *actor_key)
                        });
                        state.history.retain(|row| {
                            !(row.stored.message_id == *message_id && row.actor_key == *actor_key)
                        });
                    }
                }
            }
            state.trim(append.capacity);
            state.trim_history(append.history_capacity);
            state.sync_token = Some(append.next_sync_token.clone());
            NetworkAgentInboxAppendOutcome::Applied { appended }
        } else {
            NetworkAgentInboxAppendOutcome::Stale
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn acknowledge<'a>(
        &'a self,
        _id: NetworkAgentId,
        event_id: &'a MatrixEventId,
        room: Option<&'a MatrixRoomId>,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentAckOutcome>> {
        let mut state = self.state.lock().unwrap();
        let sequence = state
            .entries
            .iter()
            .find(|row| row.event_id == *event_id && row.in_scope(room))
            .map(|row| row.sequence);
        let outcome = if let Some(sequence) = sequence {
            state
                .entries
                .retain(|row| row.sequence > sequence || !row.in_scope(room));
            state.dropped = 0;
            NetworkAgentAckOutcome::Acknowledged {
                pending: state.pending(room),
            }
        } else {
            NetworkAgentAckOutcome::NotPending {
                pending: state.pending(room),
            }
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn prune_expired(
        &self,
        now: UtcMillis,
        retention: NetworkAgentMessageRetention,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        self.pruned.lock().unwrap().push((now, retention));
        Box::pin(async { Ok(0) })
    }
}

impl NetworkAgentMessageHistory for MemoryInbox {
    fn messages_by_id<'a>(
        &'a self,
        _id: NetworkAgentId,
        refs: &'a [NetworkAgentMessageRef],
    ) -> PortFuture<'a, RepositoryResult<Vec<NetworkAgentStoredMessage>>> {
        let state = self.state.lock().unwrap();
        let found = state
            .history
            .iter()
            .filter(|row| {
                refs.iter().any(|reference| match reference {
                    NetworkAgentMessageRef::Event(event) => row.stored.event_id == *event,
                    NetworkAgentMessageRef::Message(id) => row.stored.message_id == *id,
                })
            })
            .map(|row| row.stored.clone())
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn room_messages<'a>(
        &'a self,
        _id: NetworkAgentId,
        room: &'a MatrixRoomId,
        direction: NetworkAgentHistoryDirection,
        filter: &'a NetworkAgentHistoryFilter,
        limit: u16,
    ) -> PortFuture<'a, RepositoryResult<Vec<NetworkAgentStoredMessage>>> {
        let state = self.state.lock().unwrap();
        let in_room = state
            .history
            .iter()
            .filter(|row| row.stored.room_id == *room && row.wanted(filter));
        let picked: Vec<NetworkAgentStoredMessage> = match direction {
            NetworkAgentHistoryDirection::Before(before) => in_room
                .rev()
                .filter(|row| before.is_none_or(|before| row.stored.sequence < before))
                .take(usize::from(limit))
                .map(|row| row.stored.clone())
                .collect(),
            NetworkAgentHistoryDirection::After(after) => in_room
                .filter(|row| row.stored.sequence > after)
                .take(usize::from(limit))
                .map(|row| row.stored.clone())
                .collect(),
        };
        Box::pin(async move { Ok(picked) })
    }
}

enum Step {
    Batch(MatrixResult<MatrixSyncBatch>),
    /// 一直等到被通知才返回空批次，模拟 Matrix 长轮询。
    Block(Arc<Notify>),
    /// 过一会儿才来：长轮询中途房间里有了动静。
    Later(Duration, MatrixSyncBatch),
}

/// 按顺序给出同步结果；用完之后按请求的超时等一会儿再返回空批次。发言按事务 ID 去重，
/// 像 Synapse 一样：同一事务 ID 重发拿到同一个事件 ID。
#[derive(Default)]
struct ScriptedMatrix {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<NetworkAgentSyncRequest>>,
    tokens: Mutex<Vec<String>>,
    sent: Mutex<Vec<(MatrixRoomId, MatrixEvent)>>,
    send_failures: Mutex<VecDeque<MatrixFailureKind>>,
    states: Mutex<Vec<(String, Value)>>,
    /// 读房间里的那条状态时一律失败。
    state_read_fails: Mutex<bool>,
    left: Mutex<Vec<String>>,
    leave_fails: Mutex<bool>,
    /// 往回翻的结果，按顺序给出；给完之后都是“翻到了房间最早的历史”。
    backfills: Mutex<VecDeque<MatrixResult<MatrixBackfillPage>>>,
    /// 往回翻过的房间、令牌和条数。
    backfilled: Mutex<Vec<(String, String, u16)>>,
    /// 报过的 Matrix 在线状态。`None` 是这台假服务器不接在线状态的接口，网关照旧写租约。
    presence: Mutex<Option<Vec<MatrixPresenceState>>>,
}

impl ScriptedMatrix {
    fn push(&self, step: Step) {
        self.steps.lock().unwrap().push_back(step);
    }

    fn requests(&self) -> Vec<NetworkAgentSyncRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn sent(&self) -> Vec<(MatrixRoomId, MatrixEvent)> {
        self.sent.lock().unwrap().clone()
    }

    fn backfill_with(&self, page: MatrixResult<MatrixBackfillPage>) {
        self.backfills.lock().unwrap().push_back(page);
    }

    /// 开着在线状态：报的记下来，读回自己时答最后报的那个。
    fn enable_presence(&self) {
        *self.presence.lock().unwrap() = Some(Vec::new());
    }

    fn reported_presence(&self) -> Vec<MatrixPresenceState> {
        self.presence.lock().unwrap().clone().unwrap_or_default()
    }

    fn backfilled(&self) -> Vec<(String, String, u16)> {
        self.backfilled.lock().unwrap().clone()
    }
}

/// 往回翻一页：`events` 新的在前；没有 `end` 就是翻到了房间最早的历史。
fn backfill_page(
    start: &str,
    end: Option<&str>,
    events: Vec<MatrixTimelineEvent>,
) -> MatrixBackfillPage {
    MatrixBackfillPage::new(
        MatrixBackfillToken::new(start).unwrap(),
        end.map(|token| MatrixBackfillToken::new(token).unwrap()),
        events,
    )
}

/// 照着往回翻的请求记一笔、按顺序给出下一页。
fn next_backfill(
    queue: &Mutex<VecDeque<MatrixResult<MatrixBackfillPage>>>,
    seen: &Mutex<Vec<(String, String, u16)>>,
    room_id: &MatrixRoomId,
    request: &MatrixBackfillRequest,
) -> MatrixResult<MatrixBackfillPage> {
    seen.lock().unwrap().push((
        room_id.as_str().to_owned(),
        request.from().as_str().to_owned(),
        request.limit().get(),
    ));
    queue
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| Ok(backfill_page(request.from().as_str(), None, Vec::new())))
}

impl NetworkAgentMatrixGateway for ScriptedMatrix {
    fn sync<'a>(
        &'a self,
        access_token: &'a SecretValue,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>> {
        self.requests.lock().unwrap().push(request.clone());
        self.tokens
            .lock()
            .unwrap()
            .push(access_token.expose().to_owned());
        let step = self.steps.lock().unwrap().pop_front();
        let since = request
            .since
            .as_ref()
            .map_or_else(|| "s0".to_owned(), |token| token.as_str().to_owned());
        let timeout = request.timeout_millis;
        Box::pin(async move {
            match step {
                Some(Step::Batch(result)) => result,
                Some(Step::Block(notify)) => {
                    notify.notified().await;
                    Ok(batch(&format!("{since}+"), Vec::new()))
                }
                Some(Step::Later(delay, later)) => {
                    tokio::time::sleep(delay).await;
                    Ok(later)
                }
                None => {
                    tokio::time::sleep(Duration::from_millis(timeout)).await;
                    Ok(batch(&since, Vec::new()))
                }
            }
        })
    }

    fn send_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        assert_eq!(access_token.expose(), "syt_scout");
        self.sent
            .lock()
            .unwrap()
            .push((room_id.clone(), event.clone()));
        let failure = self.send_failures.lock().unwrap().pop_front();
        let result = match failure {
            Some(kind) => Err(MatrixFailure::new(MatrixOperation::SendEvent, kind)),
            None => Ok(MatrixAcceptedEvent::new(
                event.transaction_id().clone(),
                MatrixEventId::new(format!(
                    "$sent-{}:matrix.test",
                    event.transaction_id().as_str()
                ))
                .unwrap(),
            )),
        };
        Box::pin(async move { result })
    }

    fn send_state_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        assert_eq!(access_token.expose(), "syt_scout");
        assert_eq!(
            event.event_type().as_str(),
            "io.github.rainyflash.agentroom.agent.status.v1"
        );
        assert_eq!(event.state_key().as_str(), OWN_INSTANCE);
        let mut states = self.states.lock().unwrap();
        states.push((room_id.as_str().to_owned(), event.content().clone()));
        let event_id = MatrixEventId::new(format!("$status-{}:matrix.test", states.len())).unwrap();
        Box::pin(async move { Ok(event_id) })
    }

    fn leave<'a>(
        &'a self,
        _access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.left.lock().unwrap().push(room_id.as_str().to_owned());
        let result = if *self.leave_fails.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::Leave,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(())
        };
        Box::pin(async move { result })
    }

    fn backfill<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        assert_eq!(access_token.expose(), "syt_scout");
        let page = next_backfill(&self.backfills, &self.backfilled, room_id, request);
        Box::pin(async move { page })
    }

    fn report_presence<'a>(
        &'a self,
        access_token: &'a SecretValue,
        user_id: &'a MatrixUserId,
        presence: MatrixPresenceState,
    ) -> PortFuture<'a, MatrixResult<()>> {
        assert_eq!(access_token.expose(), "syt_scout");
        assert_eq!(user_id.as_str(), matrix_user(OWN_AGENT));
        let result = match self.presence.lock().unwrap().as_mut() {
            Some(reported) => {
                reported.push(presence);
                Ok(())
            }
            None => Err(MatrixFailure::new(
                MatrixOperation::ReportPresence,
                MatrixFailureKind::NotFound,
            )),
        };
        Box::pin(async move { result })
    }

    fn own_presence<'a>(
        &'a self,
        access_token: &'a SecretValue,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixPresenceState>> {
        assert_eq!(access_token.expose(), "syt_scout");
        assert_eq!(user_id.as_str(), matrix_user(OWN_AGENT));
        let result = match self.presence.lock().unwrap().as_ref() {
            Some(reported) => Ok(reported
                .last()
                .copied()
                .unwrap_or(MatrixPresenceState::Offline)),
            None => Err(MatrixFailure::new(
                MatrixOperation::ReadPresence,
                MatrixFailureKind::NotFound,
            )),
        };
        Box::pin(async move { result })
    }

    /// 房间里这条状态最新的内容，和 Synapse 一样：写过的里面最后一条。
    fn state_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        _event_type: &'a MatrixEventType,
        state_key: &'a MatrixStateKey,
    ) -> PortFuture<'a, MatrixResult<Option<Value>>> {
        assert_eq!(access_token.expose(), "syt_scout");
        assert_eq!(state_key.as_str(), OWN_INSTANCE);
        if *self.state_read_fails.lock().unwrap() {
            return Box::pin(async {
                Err(MatrixFailure::new(
                    MatrixOperation::ReadRoomState,
                    MatrixFailureKind::Timeout,
                ))
            });
        }
        let latest = self
            .states
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(room, _)| room == room_id.as_str())
            .map(|(_, content)| content.clone());
        Box::pin(async move { Ok(latest) })
    }
}

/// 验签查到的实例：另一个 Agent 与自己的实例都登记过。
struct KnownInstances;

impl AgentInstanceVerificationRepository for KnownInstances {
    fn find_verification_record(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceVerificationRecord>>> {
        let agent_id = if instance_id == instance(OTHER_INSTANCE) {
            Some(agent(OTHER_AGENT))
        } else if instance_id == instance(OWN_INSTANCE) {
            Some(agent(OWN_AGENT))
        } else {
            None
        };
        let record = agent_id.map(|agent_id| AgentInstanceVerificationRecord {
            instance_id,
            agent_id,
            public_signing_key: AgentInstancePublicSigningKey::new(vec![7; 32]).unwrap(),
            registered_at: UtcMillis::new(1).unwrap(),
            invalidated_at: None,
        });
        Box::pin(async move { Ok(record) })
    }
}

struct FakeSignatures;

impl AgentInstanceSignatureVerifier for FakeSignatures {
    fn verify(
        &self,
        _public_key: &AgentInstancePublicSigningKey,
        _signed_message: &[u8],
        signature: &DeviceSignature,
    ) -> bool {
        signature.as_bytes() != &FORGED_SIGNATURE
    }
}

/// 加密客户端替身：记下同步请求，按顺序给出结果；用完之后按请求的超时等一会儿再返回空批次。
#[derive(Default)]
struct FakeEncrypted {
    batches: Mutex<VecDeque<Result<MatrixSyncBatch, NetworkGatewayFailure>>>,
    requests: Mutex<Vec<(NetworkAgentId, NetworkAgentSyncRequest)>>,
    /// 准备时带的会话里进加密房间的时刻。
    prepared: Mutex<Vec<Option<UtcMillis>>>,
    prepare_failure: Mutex<Option<NetworkGatewayFailure>>,
    /// 完整同步了几次。
    refreshed: Mutex<u32>,
    /// 发言用的加密客户端替身。
    client: Arc<FakeClient>,
    forgotten: Mutex<Vec<NetworkAgentId>>,
    /// 删掉了加密存储的；`remove_fails` 为真时删不掉。
    removed: Mutex<Vec<NetworkAgentId>>,
    remove_fails: Mutex<bool>,
    /// 下一轮清理时关掉几个闲置的。
    idle: Mutex<usize>,
    /// 与用例替身共用，看先后。
    log: Arc<Mutex<Vec<String>>>,
    /// 往回翻的结果，按顺序给出；给完之后都是“翻到了房间最早的历史”。
    backfills: Mutex<VecDeque<MatrixResult<MatrixBackfillPage>>>,
    backfilled: Mutex<Vec<(String, String, u16)>>,
}

impl FakeEncrypted {
    fn requests(&self) -> Vec<(NetworkAgentId, NetworkAgentSyncRequest)> {
        self.requests.lock().unwrap().clone()
    }
}

impl EncryptedSessions for FakeEncrypted {
    fn sync<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, Result<MatrixSyncBatch, NetworkGatewayFailure>> {
        self.requests
            .lock()
            .unwrap()
            .push((session.network_agent_id, request.clone()));
        let next = self.batches.lock().unwrap().pop_front();
        let since = request
            .since
            .as_ref()
            .map_or_else(|| "e0".to_owned(), |token| token.as_str().to_owned());
        let timeout = request.timeout_millis;
        Box::pin(async move {
            if let Some(result) = next {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(timeout)).await;
            Ok(batch(&since, Vec::new()))
        })
    }

    fn backfill<'a>(
        &'a self,
        _session: &'a NetworkAgentSession,
        room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        let page = next_backfill(&self.backfills, &self.backfilled, room_id, request);
        Box::pin(async move { page })
    }

    fn prepare<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        self.log.lock().unwrap().push("prepare".to_owned());
        self.prepared.lock().unwrap().push(session.encrypted_since);
        let failure = self.prepare_failure.lock().unwrap().clone();
        Box::pin(async move { failure.map_or(Ok(()), Err) })
    }

    fn refresh<'a>(
        &'a self,
        _session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        self.log.lock().unwrap().push("refresh".to_owned());
        *self.refreshed.lock().unwrap() += 1;
        Box::pin(async { Ok(()) })
    }

    fn speaker<'a>(
        &'a self,
        _session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<EncryptedSpeaker, NetworkGatewayFailure>> {
        let speaker = EncryptedSpeaker {
            matrix: self.client.clone(),
            authority: self.client.clone(),
            security: self.client.clone(),
            protection: Arc::new(MessageBodyProtectionService::new(Arc::new(
                AesGcmMessageContentCipher::new(MessageContentRootKey::from_bytes([7; 32])),
            ))),
        };
        Box::pin(async move { Ok(speaker) })
    }

    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()> {
        self.forgotten.lock().unwrap().push(id);
        Box::pin(async {})
    }

    fn remove_store(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, Result<(), NetworkGatewayFailure>> {
        self.log.lock().unwrap().push("remove_store".to_owned());
        let result = if *self.remove_fails.lock().unwrap() {
            Err(NetworkGatewayFailure::Unavailable)
        } else {
            self.removed.lock().unwrap().push(id);
            Ok(())
        };
        Box::pin(async move { result })
    }

    fn evict_idle(&self) -> PortFuture<'_, usize> {
        let idle = std::mem::take(&mut *self.idle.lock().unwrap());
        Box::pin(async move { idle })
    }
}

/// 加密客户端替身：发事件、说房间加不加密、按顺序给出“房间就绪”的结果（给完之后都算就绪）。
struct FakeClient {
    metadata: MatrixSessionMetadata,
    encryption: Mutex<MatrixRoomEncryption>,
    readiness: Mutex<VecDeque<Result<(), MatrixSecurityFailure>>>,
    ensured: Mutex<u32>,
    sent: Mutex<Vec<(MatrixRoomId, MatrixEvent)>>,
}

impl Default for FakeClient {
    fn default() -> Self {
        Self {
            metadata: MatrixSessionMetadata::new(
                MatrixUserId::new(matrix_user(OWN_AGENT)).unwrap(),
                MatrixDeviceId::new(format!("AR_{}", uuid(OWN_INSTANCE).simple())).unwrap(),
            ),
            encryption: Mutex::new(MatrixRoomEncryption::EndToEnd),
            readiness: Mutex::new(VecDeque::new()),
            ensured: Mutex::new(0),
            sent: Mutex::new(Vec::new()),
        }
    }
}

impl MatrixGateway for FakeClient {
    fn metadata(&self) -> &MatrixSessionMetadata {
        &self.metadata
    }

    fn sync_once<'a>(
        &'a self,
        _request: &'a MatrixSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>> {
        unreachable!("同步走 EncryptedSessions 替身")
    }

    fn create_room<'a>(
        &'a self,
        _request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        unreachable!("网络 Agent 不建房间")
    }

    fn resolve_room_alias<'a>(
        &'a self,
        _alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        unreachable!("网络 Agent 不解析别名")
    }

    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("网络 Agent 不邀请")
    }

    fn join<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("进房间走用例")
    }

    fn leave<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("离开走轻量客户端")
    }

    fn send_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        self.sent
            .lock()
            .unwrap()
            .push((room_id.clone(), event.clone()));
        let accepted = MatrixAcceptedEvent::new(
            event.transaction_id().clone(),
            MatrixEventId::new(format!(
                "$encrypted-{}:matrix.test",
                event.transaction_id().as_str()
            ))
            .unwrap(),
        );
        Box::pin(async move { Ok(accepted) })
    }

    fn send_state_event<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        unreachable!("在线状态走轻量客户端")
    }

    fn send_receipt<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _receipt: &'a MatrixReceipt,
    ) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("网络 Agent 不发回执")
    }

    fn backfill<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        unreachable!("网络 Agent 不回填")
    }
}

impl MatrixRoomAuthorityGateway for FakeClient {
    fn inspect_room_authority<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomAuthority>> {
        let authority = MatrixRoomAuthority::joined(MatrixPowerLevel::Finite(50))
            .with_encryption(*self.encryption.lock().unwrap());
        Box::pin(async move { Ok(authority) })
    }
}

impl MatrixSecurityGateway for FakeClient {
    fn recover(
        &self,
        _command: MatrixRecoveryCommand,
    ) -> PortFuture<'_, Result<MatrixRecoveryResult, MatrixSecurityFailure>> {
        unreachable!("身份由加密客户端自己建")
    }

    fn ensure_room_ready<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<(), MatrixSecurityFailure>> {
        *self.ensured.lock().unwrap() += 1;
        let ready = self.readiness.lock().unwrap().pop_front().unwrap_or(Ok(()));
        Box::pin(async move { ready })
    }

    fn execute(
        &self,
        _command: MatrixSecurityCommand,
    ) -> PortFuture<'_, Result<MatrixSecurityResult, MatrixSecurityFailure>> {
        unreachable!("身份由加密客户端自己建")
    }
}

/// 跟着 tokio 的时间走：暂停时间的测试里，等待多久，钟就走多久。
struct TokioClock {
    started: tokio::time::Instant,
}

impl Clock for TokioClock {
    fn now(&self) -> UtcMillis {
        let elapsed = i64::try_from(self.started.elapsed().as_millis()).unwrap();
        UtcMillis::new(1_758_600_000_000 + elapsed).unwrap()
    }
}

// ---------- 组装与构造 ----------

struct Harness {
    gateway: NetworkGateway,
    agents: Arc<FakeAgents>,
    inbox: Arc<MemoryInbox>,
    submissions: Arc<MemorySubmissions>,
    content: Arc<FakeContent>,
    matrix: Arc<ScriptedMatrix>,
    encrypted: Arc<FakeEncrypted>,
}

fn harness() -> Harness {
    harness_in(&[ROOM])
}

fn harness_in(rooms: &[&str]) -> Harness {
    build_harness(rooms, true)
}

fn build_harness(rooms: &[&str], encrypted_clients: bool) -> Harness {
    harness_with(FakeAgents::in_rooms(rooms), encrypted_clients)
}

fn harness_with(agents: FakeAgents, encrypted_clients: bool) -> Harness {
    let agents = Arc::new(agents);
    let inbox = Arc::new(MemoryInbox::default());
    let submissions = Arc::new(MemorySubmissions::default());
    let content = Arc::new(FakeContent::default());
    let matrix = Arc::new(ScriptedMatrix::default());
    let encrypted = Arc::new(FakeEncrypted {
        log: agents.log.clone(),
        ..FakeEncrypted::default()
    });
    let gateway = NetworkGateway::new(NetworkGatewayDependencies {
        agents: agents.clone(),
        inbox: inbox.clone(),
        history: inbox.clone(),
        submissions: submissions.clone(),
        matrix: matrix.clone(),
        content: content.clone(),
        verification: Arc::new(KnownInstances),
        signatures: Arc::new(FakeSignatures),
        clock: Arc::new(TokioClock {
            started: tokio::time::Instant::now(),
        }),
        encrypted: encrypted_clients.then(|| encrypted.clone() as Arc<dyn EncryptedSessions>),
    });
    Harness {
        gateway,
        agents,
        inbox,
        submissions,
        content,
        matrix,
        encrypted,
    }
}

/// 同样的依赖再建一个网关：控制面重启了，进程里记着的都没了，库里和 Matrix 上的还在。
fn restarted(harness: &Harness) -> NetworkGateway {
    NetworkGateway::new(NetworkGatewayDependencies {
        agents: harness.agents.clone(),
        inbox: harness.inbox.clone(),
        history: harness.inbox.clone(),
        submissions: harness.submissions.clone(),
        matrix: harness.matrix.clone(),
        content: harness.content.clone(),
        verification: Arc::new(KnownInstances),
        signatures: Arc::new(FakeSignatures),
        clock: Arc::new(TokioClock {
            started: tokio::time::Instant::now(),
        }),
        encrypted: Some(harness.encrypted.clone() as Arc<dyn EncryptedSessions>),
    })
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

fn network_agent_id() -> NetworkAgentId {
    NetworkAgentId::from_uuid(uuid(NETWORK_AGENT))
}

fn agent(value: &str) -> AgentId {
    AgentId::from_uuid(uuid(value))
}

fn instance(value: &str) -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid(value))
}

fn matrix_user(agent_id: &str) -> String {
    format!("@_agent_{}:matrix.test", uuid(agent_id).simple())
}

fn private_room() -> NetworkAgentRoom {
    NetworkAgentRoom {
        catalog_id: RoomCatalogId::from_uuid(uuid(PRINCIPAL)),
        matrix_room_id: MatrixRoomReference::new(PRIVATE_ROOM.to_owned()).unwrap(),
        name: "项目室".to_owned(),
    }
}

fn knock() -> NetworkAgentKnock {
    NetworkAgentKnock {
        catalog_id: private_room().catalog_id,
        status: NetworkAgentKnockStatus::Waiting,
        knocked_at: UtcMillis::new(1_000).unwrap(),
        expires_at: UtcMillis::new(3_601_000).unwrap(),
    }
}

/// 进去了的房间；敲门的不算。
fn entered(entry: NetworkAgentEntry) -> NetworkAgentRoom {
    match entry {
        NetworkAgentEntry::Entered(room) => room,
        NetworkAgentEntry::Knocked(knock) => panic!("只敲了门：{knock:?}"),
    }
}

fn create_with_code() -> CreateNetworkAgent {
    CreateNetworkAgent {
        name: "Cipher".to_owned(),
        room: NetworkAgentRoomRequest::Code("K7P3-Q9XW-2DMA".to_owned()),
        source_digest: [1; 32],
    }
}

fn batch(next: &str, events: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
    let rooms = if events.is_empty() {
        Vec::new()
    } else {
        vec![MatrixRoomSync::new(
            MatrixRoomId::new(ROOM).unwrap(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            events,
            Vec::new(),
        )]
    };
    MatrixSyncBatch::new(MatrixSyncToken::new(next).unwrap(), rooms)
}

/// 这一段同步里大厅的“正在输入”变了，可以顺带几条消息。
fn typing_batch(
    next: &str,
    typing: &[String],
    events: Vec<MatrixTimelineEvent>,
) -> MatrixSyncBatch {
    let room = MatrixRoomSync::new(
        MatrixRoomId::new(ROOM).unwrap(),
        MatrixRoomSyncKind::Joined,
        false,
        None,
        events,
        Vec::new(),
    )
    .with_typing(
        typing
            .iter()
            .map(|user| MatrixUserId::new(user.as_str()).unwrap())
            .collect(),
    );
    MatrixSyncBatch::new(MatrixSyncToken::new(next).unwrap(), vec![room])
}

/// 几个房间的一次同步。
fn rooms_batch(next: &str, rooms: Vec<(&str, Vec<MatrixTimelineEvent>)>) -> MatrixSyncBatch {
    MatrixSyncBatch::new(
        MatrixSyncToken::new(next).unwrap(),
        rooms
            .into_iter()
            .map(|(room, events)| {
                MatrixRoomSync::new(
                    MatrixRoomId::new(room).unwrap(),
                    MatrixRoomSyncKind::Joined,
                    false,
                    None,
                    events,
                    Vec::new(),
                )
            })
            .collect(),
    )
}

/// 同一条消息，改成在另一个房间里发的。
fn in_room(event: &MatrixTimelineEvent, room: &str) -> MatrixTimelineEvent {
    let mut content = event.content().clone();
    content["roomId"] = json!(room);
    MatrixTimelineEvent::new(
        event.event_id().cloned(),
        event.sender().cloned(),
        event.event_type().clone(),
        None,
        None,
        Some(1_758_600_000_000),
        content,
    )
    .unwrap()
}

fn signature(bytes: [u8; 64]) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn actor(agent_id: &str, instance_id: &str) -> Value {
    json!({
        "agent": {
            "agentId": agent_id,
            "displayName": if agent_id == OWN_AGENT { "Scout" } else { "Ranger" },
            "matrixUserId": matrix_user(agent_id),
        },
        "instanceId": instance_id,
        "provenance": "autonomous_agent",
    })
}

/// 一条聊天消息的预览事件。
fn chat(
    event_id: &str,
    from: (&str, &str),
    message_id: Uuid,
    text: &str,
    signature_bytes: [u8; 64],
) -> MatrixTimelineEvent {
    let content = json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.preview.v1",
        "id": message_id,
        "createdAt": "2026-09-23T12:00:00.000Z",
        "actor": actor(from.0, from.1),
        "correlationId": message_id,
        "roomId": ROOM,
        "preview": {
            "conversation": {"text": text, "mentions": []},
            "title": text,
            "summary": text,
            "contentType": "text/plain",
            "sensitivity": "normal",
            "riskFlags": [],
        },
        "content": {
            "contentId": Uuid::now_v7(),
            "digestSha256": "11".repeat(32),
            "sizeBytes": 16,
            "mediaType": "text/plain",
            "fetchMode": "on_demand",
        },
        "signature": signature(signature_bytes),
    });
    event(
        event_id,
        from.0,
        "io.github.rainyflash.agentroom.message.preview.v1",
        content,
    )
}

/// 带点名和回复的聊天消息。
fn chat_with(
    event_id: &str,
    from: (&str, &str),
    message_id: Uuid,
    text: &str,
    mentions: &[String],
    reply_to: Option<Uuid>,
) -> MatrixTimelineEvent {
    let mut event = chat(event_id, from, message_id, text, [1; 64]);
    let mut content = event.content().clone();
    content["preview"]["conversation"]["mentions"] = json!(mentions);
    // 标题、摘要有长度上限：长正文只拿开头当标题和摘要，像真实发送方那样。
    content["preview"]["title"] = json!(text.chars().take(120).collect::<String>());
    content["preview"]["summary"] = json!(text.chars().take(500).collect::<String>());
    if let Some(target) = reply_to {
        content["relation"] = json!({"kind": "reply", "targetMessageId": target});
    }
    event = MatrixTimelineEvent::new(
        event.event_id().cloned(),
        event.sender().cloned(),
        event.event_type().clone(),
        None,
        None,
        Some(1_758_600_000_000),
        content,
    )
    .unwrap();
    event
}

fn revision(event_id: &str, from: (&str, &str), target: Uuid, kind: &str) -> MatrixTimelineEvent {
    let id = Uuid::now_v7();
    let mut content = json!({
        "schemaVersion": "1.0",
        "eventType": "io.github.rainyflash.agentroom.message.revision.v1",
        "id": id,
        "createdAt": "2026-09-23T12:01:00.000Z",
        "actor": actor(from.0, from.1),
        "correlationId": id,
        "roomId": ROOM,
        "targetMessageId": target,
        "kind": kind,
        "signature": signature([1; 64]),
    });
    if kind == "replace" {
        content["preview"] = json!({
            "conversation": {"text": "改过的话", "mentions": []},
            "title": "改过的话",
            "summary": "改过的话",
            "contentType": "text/plain",
            "sensitivity": "normal",
            "riskFlags": [],
        });
        content["content"] = json!({
            "contentId": Uuid::now_v7(),
            "digestSha256": "22".repeat(32),
            "sizeBytes": 12,
            "mediaType": "text/plain",
            "fetchMode": "on_demand",
        });
    }
    event(
        event_id,
        from.0,
        "io.github.rainyflash.agentroom.message.revision.v1",
        content,
    )
}

fn event(
    event_id: &str,
    sender_agent: &str,
    event_type: &str,
    content: Value,
) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new(matrix_user(sender_agent)).unwrap()),
        MatrixEventType::new(event_type).unwrap(),
        None,
        None,
        Some(1_758_600_000_000),
        content,
    )
    .unwrap()
}

fn other() -> (&'static str, &'static str) {
    (OTHER_AGENT, OTHER_INSTANCE)
}

fn own() -> (&'static str, &'static str) {
    (OWN_AGENT, OWN_INSTANCE)
}

/// 原来的取法：别人说的都叫醒，来了立刻交。专门测叫醒规则的用例自己写选项。
fn everything(wait: Duration, limit: u16) -> NetworkAgentWait {
    NetworkAgentWait {
        wait,
        limit,
        options: WaitOptions {
            wake: WakeRule::All,
            settle: Duration::ZERO,
            ..WaitOptions::default()
        },
        wait_for_mentioned: false,
        room: None,
    }
}

/// 按默认规则等：跟它有关的才叫醒，等对话停 5 秒再交。
fn related(wait: Duration) -> NetworkAgentWait {
    NetworkAgentWait {
        wait,
        limit: 20,
        options: WaitOptions::default(),
        wait_for_mentioned: false,
        room: None,
    }
}

fn texts(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message["conversation"]["text"].as_str().unwrap())
        .collect()
}

// ---------- 用例 ----------

#[tokio::test(start_paused = true)]
async fn 第一次不等_带回最近的几条_自己发的不进收件箱_没确认前再取还是这些() {
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat(
                "$hello:matrix.test",
                other(),
                Uuid::now_v7(),
                "你好",
                [1; 64],
            ),
            chat(
                "$mine:matrix.test",
                own(),
                Uuid::now_v7(),
                "我自己说的",
                [1; 64],
            ),
        ],
    ))));

    let first = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .expect("取到消息");

    assert_eq!(texts(&first.messages), ["你好"]);
    assert_eq!(first.pending, 1);
    let message = &first.messages[0];
    assert_eq!(message["eventId"], "$hello:matrix.test");
    assert_eq!(message["roomId"], ROOM);
    assert_eq!(message["actor"]["kind"], "agent");
    assert_eq!(message["actor"]["agent"]["displayName"], "Ranger");
    assert_eq!(message["actor"]["provenance"], "autonomous_agent");

    let requests = harness.matrix.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].since, None);
    assert_eq!(requests[0].timeout_millis, 0);
    assert_eq!(requests[0].timeline_limit, 20);
    assert_eq!(*harness.matrix.tokens.lock().unwrap(), ["syt_scout"]);
    assert_eq!(harness.inbox.sync_token().as_deref(), Some("s1"));

    // 没确认，再取还是这一条；同时不等待地问一次 Matrix，新到的照样进收件箱。
    let again = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .expect("再取");
    assert_eq!(texts(&again.messages), ["你好"]);
    let requests = harness.matrix.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].since.as_ref().unwrap().as_str(), "s1");
    assert_eq!(requests[1].timeout_millis, 0);
}

#[tokio::test(start_paused = true)]
async fn 还有没确认的也照样把新消息取进收件箱_不会因为积压悄悄丢() {
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![chat(
            "$one:matrix.test",
            other(),
            Uuid::now_v7(),
            "一",
            [1; 64],
        )],
    ))));
    let first = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    assert_eq!(texts(&first.messages), ["一"]);

    // Agent 还在处理第一条时房间里又来了一条：再取时一并取进来，不必等它先确认。原来要等收件箱
    // 清空才问 Matrix，一个房间里积下的超过一次同步能带回的条数，更早的就悄悄丢了。
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$two:matrix.test",
            other(),
            Uuid::now_v7(),
            "二",
            [1; 64],
        )],
    ))));
    let started = tokio::time::Instant::now();
    let again = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    assert_eq!(texts(&again.messages), ["一", "二"]);
    assert_eq!(again.pending, 2);
    assert_eq!(started.elapsed(), Duration::ZERO, "收件箱里有消息时不等");
    let requests = harness.matrix.requests();
    assert_eq!(requests[1].since.as_ref().unwrap().as_str(), "s1");
    assert_eq!(requests[1].timeout_millis, 0);
    assert_eq!(requests[1].timeline_limit, 50);
    assert_eq!(harness.inbox.sync_token().as_deref(), Some("s2"));
}

#[tokio::test(start_paused = true)]
async fn 还有没确认的时同步失败_照样先交出已有的() {
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![chat(
            "$one:matrix.test",
            other(),
            Uuid::now_v7(),
            "一",
            [1; 64],
        )],
    ))));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();

    harness.matrix.push(Step::Batch(Err(MatrixFailure::new(
        MatrixOperation::Sync,
        MatrixFailureKind::DependencyUnavailable,
    ))));
    let again = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .expect("Matrix 暂时不通也先交出收件箱里的");
    assert_eq!(texts(&again.messages), ["一"]);
}

#[tokio::test(start_paused = true)]
async fn 标出点名自己和回复自己的_附上被回复那条的开头_长正文只给开头() {
    let harness = harness();
    let mine = Uuid::now_v7();
    let long = "字".repeat(1_500);
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat("$mine:matrix.test", own(), mine, "我先来", [1; 64]),
            chat_with(
                "$reply:matrix.test",
                other(),
                Uuid::now_v7(),
                "同意",
                &[],
                Some(mine),
            ),
            chat_with(
                "$named:matrix.test",
                other(),
                Uuid::now_v7(),
                "Scout 你看呢",
                &[matrix_user(OWN_AGENT)],
                None,
            ),
            chat_with(
                "$long:matrix.test",
                other(),
                Uuid::now_v7(),
                &long,
                &[],
                None,
            ),
        ],
    ))));

    let page = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    // 自己发的不进收件箱，但同一批里能拿来给回复它的那条附上摘录。
    assert_eq!(page.messages.len(), 3);
    let reply = &page.messages[0];
    assert_eq!(reply["fromMe"], false);
    assert_eq!(reply["mentionsMe"], true, "回复的是我发的");
    assert_eq!(reply["replyTo"]["messageId"], mine.to_string());
    assert_eq!(reply["replyTo"]["actorName"], "Scout");
    assert_eq!(reply["replyTo"]["excerpt"], "我先来");

    let named = &page.messages[1];
    assert_eq!(named["mentionsMe"], true);
    assert!(named.get("replyTo").is_none());

    // 超过 1000 字只给开头，全文按 ID 取。
    let long_message = &page.messages[2];
    assert_eq!(long_message["mentionsMe"], false);
    assert_eq!(
        long_message["conversation"]["text"],
        "字".repeat(1_000).as_str()
    );
    assert_eq!(long_message["conversation"]["truncated"], true);
    assert_eq!(long_message["conversation"]["fullLength"], 1_500);
    let full = harness
        .gateway
        .get_messages(TOKEN, vec!["$long:matrix.test".to_owned()])
        .await
        .unwrap();
    assert_eq!(full.messages[0]["conversation"]["text"], long.as_str());
}

#[tokio::test(start_paused = true)]
async fn 确认之后不再收到_没有新消息时等满给的时间再空手返回() {
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat("$one:matrix.test", other(), Uuid::now_v7(), "一", [1; 64]),
            chat("$two:matrix.test", other(), Uuid::now_v7(), "二", [1; 64]),
        ],
    ))));
    let first = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    assert_eq!(texts(&first.messages), ["一", "二"]);

    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$one:matrix.test", None)
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 1 }
    );
    let rest = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    assert_eq!(texts(&rest.messages), ["二"]);

    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$two:matrix.test", None)
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 0 }
    );
    // 确认过的再确认一次不报错，只说明它不在收件箱里。
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$two:matrix.test", None)
            .await
            .unwrap(),
        NetworkAgentAckOutcome::NotPending { pending: 0 }
    );

    let started = tokio::time::Instant::now();
    let empty = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    let requests = harness.matrix.requests();
    // requests[1] 是取剩下那条时不等待的那一次同步。
    assert_eq!(requests[1].timeout_millis, 0);
    assert_eq!(requests[2].since.as_ref().unwrap().as_str(), "s1");
    assert_eq!(requests[2].timeout_millis, 5_000);
    assert_eq!(requests[2].timeline_limit, 50);
}

/// 大厅里三条（最后一条点了我），二号房里一条。
fn two_rooms(harness: &Harness) {
    harness.matrix.push(Step::Batch(Ok(rooms_batch(
        "s1",
        vec![
            (
                ROOM,
                vec![
                    chat(
                        "$l1:matrix.test",
                        other(),
                        Uuid::now_v7(),
                        "大厅一",
                        [1; 64],
                    ),
                    chat(
                        "$l2:matrix.test",
                        other(),
                        Uuid::now_v7(),
                        "大厅二",
                        [1; 64],
                    ),
                    chat_with(
                        "$l3:matrix.test",
                        other(),
                        Uuid::now_v7(),
                        "Scout 你看呢",
                        &[matrix_user(OWN_AGENT)],
                        None,
                    ),
                ],
            ),
            (
                SECOND_ROOM,
                vec![in_room(
                    &chat(
                        "$s1:matrix.test",
                        other(),
                        Uuid::now_v7(),
                        "二号房",
                        [1; 64],
                    ),
                    SECOND_ROOM,
                )],
            ),
        ],
    ))));
}

#[tokio::test(start_paused = true)]
async fn 可以只看只确认一个房间_remaining_是交出去的之后还没确认的() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    two_rooms(&harness);
    let second_only = NetworkAgentWait {
        room: Some(SECOND_ROOM.to_owned()),
        ..everything(Duration::from_secs(30), 20)
    };
    let first = harness
        .gateway
        .wait_for_messages(TOKEN, second_only)
        .await
        .unwrap();
    assert_eq!(texts(&first.messages), ["二号房"]);
    assert_eq!((first.pending, first.remaining), (1, 0));
    // 只确认二号房的：大厅里更早到的还在。
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$s1:matrix.test", Some(SECOND_ROOM))
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 0 }
    );

    let one = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 1))
        .await
        .unwrap();
    assert_eq!(texts(&one.messages), ["大厅一"]);
    assert_eq!((one.pending, one.remaining), (3, 2));
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$l3:matrix.test", None)
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 0 }
    );
}

#[tokio::test(start_paused = true)]
async fn 只看一眼时只要点我的也只给点我的_不在的房间不能只看只确认() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    two_rooms(&harness);
    let mentions = NetworkAgentWait {
        options: WaitOptions {
            wake: WakeRule::Mentions,
            mentions_only: true,
            ..WaitOptions::default()
        },
        ..everything(Duration::ZERO, 20)
    };
    let named = harness
        .gateway
        .wait_for_messages(TOKEN, mentions)
        .await
        .unwrap();
    // 前面没点我的两条算跳过；二号房那条在它之后，下次再给。
    assert_eq!(texts(&named.messages), ["Scout 你看呢"]);
    assert_eq!((named.skipped, named.remaining), (2, 1));

    let elsewhere = NetworkAgentWait {
        room: Some("!elsewhere:matrix.test".to_owned()),
        ..everything(Duration::ZERO, 20)
    };
    assert_eq!(
        harness
            .gateway
            .wait_for_messages(TOKEN, elsewhere)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::RoomNotJoined
    );
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$l3:matrix.test", Some("!elsewhere:matrix.test"))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::RoomNotJoined
    );
}

#[tokio::test(start_paused = true)]
async fn 只看一眼也不等待地问一次_matrix_新到的照样取到() {
    let harness = harness();
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    let empty = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(harness.matrix.requests().len(), 1, "第一次总要同步一次");

    // 原来有了位置以后 wait=0 就不再问 Matrix，只用 wait=0 轮询的 Agent 再也收不到新消息。
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$late:matrix.test",
            other(),
            Uuid::now_v7(),
            "后来的",
            [1; 64],
        )],
    ))));
    let started = tokio::time::Instant::now();
    let later = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    assert_eq!(texts(&later.messages), ["后来的"]);
    assert_eq!(started.elapsed(), Duration::ZERO);
    let requests = harness.matrix.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].since.as_ref().unwrap().as_str(), "s1");
    assert_eq!(requests[1].timeout_millis, 0);
}

#[tokio::test(start_paused = true)]
async fn 伪造签名与冒名的事件被隔离_不进收件箱() {
    let harness = harness();
    let mut impostor = chat(
        "$impostor:matrix.test",
        other(),
        Uuid::now_v7(),
        "冒名",
        [1; 64],
    );
    // 自称另一个 Agent，但 Matrix 发送者是自己。
    impostor = MatrixTimelineEvent::new(
        impostor.event_id().cloned(),
        Some(MatrixUserId::new(matrix_user(OWN_AGENT)).unwrap()),
        impostor.event_type().clone(),
        None,
        None,
        impostor.origin_server_timestamp(),
        impostor.content().clone(),
    )
    .unwrap();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat(
                "$forged:matrix.test",
                other(),
                Uuid::now_v7(),
                "伪造",
                FORGED_SIGNATURE,
            ),
            impostor,
            chat(
                "$real:matrix.test",
                other(),
                Uuid::now_v7(),
                "真的",
                [1; 64],
            ),
        ],
    ))));

    let received = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();

    assert_eq!(texts(&received.messages), ["真的"]);
}

#[tokio::test(start_paused = true)]
async fn 作者修改或撤回还没确认的消息_收件箱跟着变_别人不能改() {
    let harness = harness();
    let edited = Uuid::now_v7();
    let withdrawn = Uuid::now_v7();
    let untouched = Uuid::now_v7();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            chat("$edited:matrix.test", other(), edited, "原话", [1; 64]),
            chat(
                "$withdrawn:matrix.test",
                other(),
                withdrawn,
                "撤回的话",
                [1; 64],
            ),
            chat(
                "$untouched:matrix.test",
                other(),
                untouched,
                "别人改不了",
                [1; 64],
            ),
            revision("$replace:matrix.test", other(), edited, "replace"),
            revision("$redact:matrix.test", other(), withdrawn, "redact"),
            // 自己发的修订对不上作者，什么也不改。
            revision("$hijack:matrix.test", own(), untouched, "redact"),
        ],
    ))));

    let received = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();

    assert_eq!(texts(&received.messages), ["改过的话", "别人改不了"]);
    assert_eq!(received.messages[0]["eventId"], "$edited:matrix.test");
    assert_eq!(received.messages[0]["messageId"], edited.to_string());
    assert_eq!(received.messages[0]["title"], "改过的话");
}

#[tokio::test(start_paused = true)]
async fn 新的长轮询让旧的立刻空手返回() {
    let harness = Arc::new(harness());
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    let blocked = Arc::new(Notify::new());
    harness.matrix.push(Step::Block(blocked.clone()));

    let old = {
        let harness = harness.clone();
        tokio::spawn(async move {
            harness
                .gateway
                .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
                .await
        })
    };
    tokio::task::yield_now().await;
    while harness.matrix.requests().len() < 2 {
        tokio::task::yield_now().await;
    }
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$new:matrix.test",
            other(),
            Uuid::now_v7(),
            "新消息",
            [1; 64],
        )],
    ))));
    let new = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();

    let old = old.await.unwrap().unwrap();
    assert!(old.messages.is_empty(), "旧的空手返回");
    assert_eq!(texts(&new.messages), ["新消息"]);
    blocked.notify_waiters();
}

#[tokio::test(start_paused = true)]
async fn 令牌不对按网络_agent_的错误回答_matrix_失败算依赖不可用() {
    let harness = harness();
    assert_eq!(
        harness
            .gateway
            .wait_for_messages("wrong", everything(Duration::from_secs(1), 20))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::Unauthorized
        ))
    );
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "not an event id", None)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::InvalidEvent
    );

    harness.matrix.push(Step::Batch(Err(MatrixFailure::new(
        MatrixOperation::Sync,
        MatrixFailureKind::DependencyUnavailable,
    ))));
    assert_eq!(
        harness
            .gateway
            .wait_for_messages(TOKEN, everything(Duration::from_secs(1), 20))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert_eq!(harness.inbox.sync_token(), None, "失败的同步不推进位置");
}

#[tokio::test(start_paused = true)]
async fn 一个房间满了丢掉这个房间最早的并告诉_agent_丢了几条() {
    let harness = harness();
    let events = (0..505)
        .map(|index| {
            chat(
                &format!("$m{index}:matrix.test"),
                other(),
                Uuid::now_v7(),
                &format!("第 {index} 条"),
                [1; 64],
            )
        })
        .collect();
    harness.matrix.push(Step::Batch(Ok(batch("s1", events))));

    let received = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 50))
        .await
        .unwrap();

    assert_eq!(received.messages.len(), 50);
    assert_eq!(received.pending, 500);
    assert_eq!(received.dropped, 5);
    assert_eq!(received.messages[0]["conversation"]["text"], "第 5 条");
    // 一次只看最早的 200 条：没看到的 300 条也算在之后还没确认的里。
    assert_eq!(received.remaining, 450);
}

// ---------- 等消息的规则 ----------

/// 第一次取消息只建立同步位置。
async fn settle_in(harness: &Harness) {
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    let first = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    assert!(first.messages.is_empty());
}

#[tokio::test(start_paused = true)]
async fn 默认跟它有关的消息到了才交_防抖后连同之前的一起给() {
    let harness = harness();
    settle_in(&harness).await;
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$chatter:matrix.test",
            other(),
            Uuid::now_v7(),
            "我们俩先聊",
            [1; 64],
        )],
    ))));
    harness.matrix.push(Step::Batch(Ok(batch(
        "s3",
        vec![chat_with(
            "$named:matrix.test",
            other(),
            Uuid::now_v7(),
            "Scout 你看呢",
            &[matrix_user(OWN_AGENT)],
            None,
        )],
    ))));

    let started = tokio::time::Instant::now();
    let page = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(texts(&page.messages), ["我们俩先聊", "Scout 你看呢"]);
    assert_eq!(page.wake.reason, WakeReason::Messages);
    assert_eq!(page.wake.event_ids, ["$named:matrix.test"]);
    assert_eq!(page.skipped, 0);
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(5),
        "等对话停 5 秒再交"
    );
}

#[tokio::test(start_paused = true)]
async fn 跟它无关的消息等满时间也不交_留着下次一起给_只看一眼时有什么给什么() {
    let harness = harness();
    settle_in(&harness).await;
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$chatter:matrix.test",
            other(),
            Uuid::now_v7(),
            "我们俩先聊",
            [1; 64],
        )],
    ))));

    let page = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(page.messages.is_empty());
    assert_eq!(page.wake.reason, WakeReason::Timeout);
    assert_eq!(page.pending, 1, "没叫醒它的留在收件箱里");

    let peek = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::ZERO))
        .await
        .unwrap();
    assert_eq!(texts(&peek.messages), ["我们俩先聊"]);
}

#[tokio::test(start_paused = true)]
async fn 等上一条点到的人都回了话再交() {
    let harness = harness();
    let mentioned = NetworkAgentWait {
        wait_for_mentioned: true,
        ..related(Duration::from_secs(30))
    };
    assert_eq!(
        harness
            .gateway
            .wait_for_messages(TOKEN, mentioned.clone())
            .await
            .unwrap_err(),
        NetworkGatewayFailure::InvalidWait("waitFor"),
        "没发过言时说不清等谁"
    );

    settle_in(&harness).await;
    harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                mentions: vec![matrix_user(OTHER_AGENT)],
                ..draft("Ranger 你怎么看？")
            },
        )
        .await
        .expect("发出去了");
    harness.matrix.push(Step::Batch(Ok(batch(
        "s2",
        vec![chat(
            "$answer:matrix.test",
            other(),
            Uuid::now_v7(),
            "我同意",
            [1; 64],
        )],
    ))));

    let page = harness
        .gateway
        .wait_for_messages(TOKEN, mentioned)
        .await
        .unwrap();
    assert_eq!(
        texts(&page.messages),
        ["我同意"],
        "Agent 没点它，但它在等这个人"
    );
    assert_eq!(page.wake.reason, WakeReason::AllReplied);
    assert!(page.wake.missing.is_empty());
}

#[tokio::test(start_paused = true)]
async fn 叫醒它的人在打字也算没停_停下以后再等防抖_上一次等消息时听说的也算() {
    let harness = harness();
    settle_in(&harness).await;
    let ranger = matrix_user(OTHER_AGENT);
    // 上一次等消息时听说 Ranger 开始打字，什么也没交。
    harness.matrix.push(Step::Batch(Ok(typing_batch(
        "s2",
        std::slice::from_ref(&ranger),
        Vec::new(),
    ))));
    let quiet = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(1)))
        .await
        .unwrap();
    assert!(quiet.messages.is_empty());

    // 这次他点了它，还接着打；8 秒后点了发送（先说停了，话随后到），再等防抖 5 秒。
    harness.matrix.push(Step::Batch(Ok(batch(
        "s3",
        vec![chat_with(
            "$named:matrix.test",
            other(),
            Uuid::now_v7(),
            "Scout 先看这个",
            &[matrix_user(OWN_AGENT)],
            None,
        )],
    ))));
    harness.matrix.push(Step::Later(
        Duration::from_secs(8),
        typing_batch("s4", &[], Vec::new()),
    ));
    let started = tokio::time::Instant::now();
    let page = harness
        .gateway
        .wait_for_messages(TOKEN, related(Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(texts(&page.messages), ["Scout 先看这个"]);
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(13),
        "防抖 5 秒到了还在打字，等他打完；停下以后再等 5 秒"
    );
}

/// 换一个服务器收到的时间：测加入前后。
fn received_at(event: &MatrixTimelineEvent, origin_ms: u64) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        event.event_id().cloned(),
        event.sender().cloned(),
        event.event_type().clone(),
        None,
        None,
        Some(origin_ms),
        event.content().clone(),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn 每条消息带上房间名_标出它进房间之前的() {
    let mut agents = FakeAgents::in_rooms(&[ROOM]);
    agents.rooms[0].name = Some("大厅".to_owned());
    agents.rooms[0].joined_at = UtcMillis::new(1_758_600_000_500).unwrap();
    let harness = harness_with(agents, true);
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![
            received_at(
                &chat(
                    "$old:matrix.test",
                    other(),
                    Uuid::now_v7(),
                    "进来之前",
                    [1; 64],
                ),
                1_758_600_000_000,
            ),
            received_at(
                &chat(
                    "$new:matrix.test",
                    other(),
                    Uuid::now_v7(),
                    "进来之后",
                    [1; 64],
                ),
                1_758_600_001_000,
            ),
        ],
    ))));

    let page = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    assert_eq!(texts(&page.messages), ["进来之前", "进来之后"]);
    assert_eq!(page.messages[0]["roomName"], "大厅");
    assert_eq!(page.messages[0]["beforeJoin"], true);
    assert_eq!(page.messages[1]["roomName"], "大厅");
    assert_eq!(page.messages[1]["beforeJoin"], false);
}

#[tokio::test(start_paused = true)]
async fn 消息副本记下服务器收到它的时间_不是网关同步到它的时间() {
    const TEN_DAYS: u64 = 10 * 24 * 60 * 60 * 1_000;
    let harness = harness();
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![received_at(
            &chat(
                "$old:matrix.test",
                other(),
                Uuid::now_v7(),
                "十天前说的",
                [1; 64],
            ),
            1_758_600_000_000 - TEN_DAYS,
        )],
    ))));

    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();

    let state = harness.inbox.state.lock().unwrap();
    let [row] = state.entries.as_slice() else {
        panic!("收到一条：{}", state.entries.len());
    };
    assert_eq!(row.sent_at.value(), 1_757_736_000_000);
    assert!(row.received_at.value() >= 1_758_600_000_000);
}

#[tokio::test(start_paused = true)]
async fn 删过了保留期的消息副本_没设保留期的按三十天_到期后多留一天() {
    let harness = harness();

    harness.gateway.prune_expired_messages().await.unwrap();

    assert_eq!(
        *harness.inbox.pruned.lock().unwrap(),
        [(
            UtcMillis::new(1_758_600_000_000).unwrap(),
            NetworkAgentMessageRetention {
                default_days: 30,
                grace_days: 1,
            }
        )]
    );
}

// ---------- 发言 ----------

fn draft(text: &str) -> NetworkAgentMessageDraft {
    NetworkAgentMessageDraft {
        room_id: None,
        text: text.to_owned(),
        reply_to: None,
        mentions: Vec::new(),
        mentions_everyone: false,
        submission_id: None,
    }
}

#[tokio::test]
async fn 发言走本机_bridge_同一套发布_正文进内容服务_签名后发出_再绑定到事件() {
    let harness = harness();
    let reply_to = Uuid::now_v7();
    let sent = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                reply_to: Some(reply_to.to_string()),
                mentions: vec![matrix_user(OTHER_AGENT)],
                ..draft("  大家好，\n我是 Scout。  ")
            },
        )
        .await
        .expect("发出去了");

    assert_eq!(sent.room, ROOM);
    let event_id = sent.event.clone().expect("Matrix 已确认");
    assert_eq!(sent.submission.as_uuid().get_version_num(), 7);

    let events = harness.matrix.sent();
    assert_eq!(events.len(), 1);
    let (room, event) = &events[0];
    assert_eq!(room.as_str(), ROOM);
    assert_eq!(
        event.event_type().as_str(),
        "io.github.rainyflash.agentroom.message.preview.v1"
    );
    assert_eq!(
        event.transaction_id().as_str(),
        format!("agent-room-message-{}", sent.submission)
    );
    assert_eq!(
        event_id,
        format!("$sent-{}:matrix.test", event.transaction_id().as_str())
    );
    let content = event.content();
    assert_eq!(content["id"], sent.submission.to_string());
    assert_eq!(content["roomId"], ROOM);
    assert_eq!(content["actor"]["agent"]["agentId"], OWN_AGENT);
    assert_eq!(content["actor"]["agent"]["displayName"], "Scout");
    assert_eq!(
        content["actor"]["agent"]["matrixUserId"],
        matrix_user(OWN_AGENT)
    );
    assert_eq!(content["actor"]["instanceId"], OWN_INSTANCE);
    assert_eq!(content["actor"]["provenance"], "autonomous_agent");
    assert_eq!(
        content["preview"]["conversation"]["text"],
        "  大家好，\n我是 Scout。  "
    );
    assert_eq!(
        content["preview"]["conversation"]["mentions"],
        json!([matrix_user(OTHER_AGENT)])
    );
    assert_eq!(content["preview"]["title"], "大家好， 我是 Scout。");
    assert_eq!(content["preview"]["contentType"], "text/plain");
    assert_eq!(content["relation"]["targetMessageId"], reply_to.to_string());
    assert!(
        content["signature"]
            .as_str()
            .is_some_and(|value| value.len() == 86)
    );

    // 正文归网络 Agent 的合成主体所有，由这个 Agent 发布，房间成员可读。
    let uploads = harness.content.uploads.lock().unwrap();
    assert_eq!(uploads.len(), 1);
    let (begun, bytes) = &uploads[0];
    assert_eq!(
        begun.owner_principal_id,
        PrincipalId::from_uuid(uuid(PRINCIPAL))
    );
    assert_eq!(begun.actor_agent_id, Some(agent(OWN_AGENT)));
    assert_eq!(begun.matrix_room_id.as_str(), ROOM);
    assert_eq!(begun.access_mode, ContentAccessMode::RoomMember);
    assert_eq!(begun.encryption_mode, ContentEncryptionMode::ServerSide);
    assert_eq!(bytes.as_slice(), "  大家好，\n我是 Scout。  ".as_bytes());
    assert_eq!(
        content["content"]["contentId"],
        begun.request_id.to_string()
    );

    let bindings = harness.content.bindings.lock().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].matrix_event_id.as_str(), event_id);
    assert_eq!(
        harness.submissions.state(sent.submission),
        Some(NetworkAgentSubmissionState::Bound)
    );
    assert_eq!(*harness.agents.quota_taken.lock().unwrap(), 1);
}

#[tokio::test]
async fn 同一个_submission_id_重试不重复发送_换了内容就冲突() {
    let harness = harness();
    let submission = Uuid::now_v7().to_string();
    let first = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission.clone()),
                ..draft("只说一次")
            },
        )
        .await
        .unwrap();
    let again = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission.clone()),
                ..draft("只说一次")
            },
        )
        .await
        .unwrap();

    assert_eq!(again, first);
    assert_eq!(harness.matrix.sent().len(), 1, "没有重复发送");

    let conflict = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission),
                ..draft("换了说法")
            },
        )
        .await
        .unwrap_err();
    assert_eq!(conflict, NetworkGatewayFailure::SubmissionConflict);
}

#[tokio::test]
async fn matrix_限速时让_agent_按它给的时间再发_带同一个_submission_id_重试就发出去() {
    let harness = harness();
    harness
        .matrix
        .send_failures
        .lock()
        .unwrap()
        .push_back(MatrixFailureKind::RateLimited);
    let submission = Uuid::now_v7().to_string();
    let limited = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission.clone()),
                ..draft("连发太快了")
            },
        )
        .await
        .unwrap_err();
    let NetworkGatewayFailure::Agent(failure) = limited else {
        panic!("Matrix 限速应当按限流回答，实际是 {limited:?}");
    };
    assert_eq!(failure.kind(), NetworkAgentFailureKind::RateLimited);
    assert!(
        failure
            .retry_at()
            .is_some_and(|at| at.value() >= 1_758_600_000_000 + 1_000),
        "Matrix 没说等多久时至少等一秒"
    );

    let retried = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission),
                ..draft("连发太快了")
            },
        )
        .await
        .expect("等过以后带同一个 submissionId 重试就发出去");
    assert!(retried.event.is_some());
}

#[tokio::test]
async fn matrix_没回话时返回待确认_带同一个_submission_id_重试就发出去() {
    let harness = harness();
    harness
        .matrix
        .send_failures
        .lock()
        .unwrap()
        .push_back(MatrixFailureKind::Timeout);
    let submission = Uuid::now_v7().to_string();
    let pending = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission.clone()),
                ..draft("可能已经发出去了")
            },
        )
        .await
        .expect("不知道结果也不算失败");
    assert_eq!(pending.event, None);
    assert_eq!(
        harness.submissions.state(pending.submission),
        Some(NetworkAgentSubmissionState::SubmitUnknown)
    );

    let retried = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                submission_id: Some(submission),
                ..draft("可能已经发出去了")
            },
        )
        .await
        .unwrap();
    assert!(retried.event.is_some());
    let sent = harness.matrix.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].1.transaction_id(), sent[1].1.transaction_id());
}

#[tokio::test]
async fn 房间要说清楚_不在的房间不能发() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    assert_eq!(
        harness
            .gateway
            .send_message(TOKEN, draft("发到哪？"))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::RoomRequired
    );
    assert_eq!(
        harness
            .gateway
            .send_message(
                TOKEN,
                NetworkAgentMessageDraft {
                    room_id: Some("!elsewhere:matrix.test".to_owned()),
                    ..draft("我不在那儿")
                },
            )
            .await
            .unwrap_err(),
        NetworkGatewayFailure::RoomNotJoined
    );
    let sent = harness
        .gateway
        .send_message(
            TOKEN,
            NetworkAgentMessageDraft {
                room_id: Some(SECOND_ROOM.to_owned()),
                ..draft("发到第二间")
            },
        )
        .await
        .unwrap();
    assert_eq!(sent.room, SECOND_ROOM);
}

#[tokio::test]
async fn 内容不合规时说明是哪一项_限流时什么都不发() {
    let harness = harness();
    let cases = [
        (draft("   "), "text"),
        (draft(&"字".repeat(4_001)), "text"),
        (
            NetworkAgentMessageDraft {
                mentions: (0..=agent_room_domain::messages::MAX_CONVERSATION_MENTIONS)
                    .map(|index| format!("@user{index}:matrix.test"))
                    .collect(),
                ..draft("太多提及")
            },
            "mentions",
        ),
        (
            NetworkAgentMessageDraft {
                mentions: (0..60)
                    .map(|index| format!("@{}{index}:matrix.test", "a".repeat(230)))
                    .collect(),
                ..draft("提及加起来太长")
            },
            "mentions",
        ),
        (
            NetworkAgentMessageDraft {
                mentions: vec!["not-a-user".to_owned()],
                ..draft("提及格式不对")
            },
            "mentions",
        ),
        (
            // 公开大厅不加密，@所有人 发不出去，也不扣发言次数。
            NetworkAgentMessageDraft {
                mentions_everyone: true,
                ..draft("大家好")
            },
            "mentionsEveryone",
        ),
        (
            NetworkAgentMessageDraft {
                reply_to: Some("not-a-uuid".to_owned()),
                ..draft("回复谁？")
            },
            "replyTo",
        ),
        (
            NetworkAgentMessageDraft {
                submission_id: Some(Uuid::new_v4().to_string()),
                ..draft("不是 UUIDv7")
            },
            "submissionId",
        ),
    ];
    for (draft, field) in cases {
        assert_eq!(
            harness
                .gateway
                .send_message(TOKEN, draft)
                .await
                .unwrap_err(),
            NetworkGatewayFailure::InvalidMessage(field),
            "{field}"
        );
    }
    assert_eq!(
        *harness.agents.quota_taken.lock().unwrap(),
        0,
        "不合规的不扣发言次数"
    );

    *harness.agents.quota.lock().unwrap() = Some(NetworkAgentFailure::rate_limited(
        UtcMillis::new(1_758_600_060_000).unwrap(),
    ));
    let limited = harness
        .gateway
        .send_message(TOKEN, draft("太快了"))
        .await
        .unwrap_err();
    assert!(matches!(
        limited,
        NetworkGatewayFailure::Agent(failure) if failure.kind() == NetworkAgentFailureKind::RateLimited
    ));
    assert!(harness.matrix.sent().is_empty());
    assert!(harness.content.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 停用时离开所有房间再作废令牌_离开失败也照样作废_留给定时清理() {
    let two_rooms = harness_in(&[ROOM, SECOND_ROOM]);
    two_rooms.gateway.leave_and_disable(TOKEN).await.unwrap();
    assert_eq!(*two_rooms.matrix.left.lock().unwrap(), [ROOM, SECOND_ROOM]);
    assert_eq!(*two_rooms.agents.disabled.lock().unwrap(), [TOKEN]);
    assert_eq!(
        *two_rooms.agents.rooms_left.lock().unwrap(),
        [network_agent_id()]
    );

    let failing = harness();
    *failing.matrix.leave_fails.lock().unwrap() = true;
    failing.gateway.leave_and_disable(TOKEN).await.unwrap();
    assert_eq!(*failing.agents.disabled.lock().unwrap(), [TOKEN]);
    assert!(
        failing.agents.rooms_left.lock().unwrap().is_empty(),
        "没离开成的不记，定时清理再试"
    );

    assert_eq!(
        failing
            .gateway
            .leave_and_disable("wrong")
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::Unauthorized
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn 进过加密房间的_agent_改由加密客户端同步_收件箱照旧() {
    let harness = harness();
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());
    harness
        .encrypted
        .batches
        .lock()
        .unwrap()
        .push_back(Ok(batch(
            "e1",
            vec![chat(
                "$secret:matrix.test",
                other(),
                Uuid::now_v7(),
                "只有房间里的人看得到",
                [1; 64],
            )],
        )));

    let first = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .expect("取到消息");

    assert_eq!(texts(&first.messages), ["只有房间里的人看得到"]);
    let requests = harness.encrypted.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, network_agent_id());
    assert_eq!(requests[0].1.since, None);
    assert_eq!(requests[0].1.timeout_millis, 0);
    assert_eq!(harness.inbox.sync_token().as_deref(), Some("e1"));

    // 确认之后接着从加密客户端给的位置等；轻量客户端始终不碰它，
    // 否则会把发给这台设备的房间密钥一并跳过去。
    harness
        .gateway
        .acknowledge(TOKEN, "$secret:matrix.test", None)
        .await
        .unwrap();
    let next = harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert!(next.messages.is_empty());
    let requests = harness.encrypted.requests();
    assert_eq!(
        requests
            .last()
            .unwrap()
            .1
            .since
            .as_ref()
            .map(MatrixSyncToken::as_str),
        Some("e1")
    );
    assert!(harness.matrix.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 加密客户端没配置时如实报暂时不可用_不退回轻量客户端() {
    let harness = build_harness(&[ROOM], false);
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());

    assert_eq!(
        harness
            .gateway
            .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert!(harness.matrix.requests().is_empty());
}

#[tokio::test]
async fn 停用与定时清理时关掉加密客户端_闲置的也关掉() {
    let disabled = harness();
    disabled.gateway.leave_and_disable(TOKEN).await.unwrap();
    assert_eq!(
        *disabled.encrypted.forgotten.lock().unwrap(),
        [network_agent_id()]
    );

    let cleanup = harness();
    let lost = NetworkAgentId::from_uuid(Uuid::now_v7());
    *cleanup.agents.exits.lock().unwrap() = vec![
        NetworkAgentPendingExit::Session(Box::new(cleanup.agents.own_session())),
        NetworkAgentPendingExit::Unopenable(lost),
    ];
    *cleanup.encrypted.idle.lock().unwrap() = 2;

    assert_eq!(
        cleanup.gateway.clean_up().await.unwrap(),
        NetworkAgentCleanupOutcome {
            left: 1,
            abandoned: 1,
            keys_deleted: 2,
            closed: 2,
            ..NetworkAgentCleanupOutcome::default()
        }
    );
    assert_eq!(
        *cleanup.encrypted.forgotten.lock().unwrap(),
        [network_agent_id(), lost]
    );
}

#[tokio::test(start_paused = true)]
async fn 凭口令创建时先切到加密客户端建好身份再进房间_之后收消息都走加密客户端() {
    let harness = harness();

    let created = harness
        .gateway
        .create(create_with_code())
        .await
        .expect("创建成功");

    assert_eq!(
        created.placement,
        NetworkAgentPlacement::Entered(private_room())
    );
    assert_eq!(
        harness.agents.log(),
        ["create", "mark_encrypted", "prepare", "enter", "refresh"],
        "身份建好之前不进：房间密钥只发给由主人交叉签名的设备；进了之后同步一次，进来就能发言"
    );
    assert_eq!(
        *harness.encrypted.prepared.lock().unwrap(),
        [Some(UtcMillis::new(42).unwrap())]
    );
    assert_eq!(
        *harness.agents.entered.lock().unwrap(),
        [NetworkAgentTarget::Private(private_room())]
    );
    assert_online_announced(&harness);

    // 用例替身给的会话里没有 encrypted_since：靠网关自己记着，照样走加密客户端。
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert_eq!(
        harness.encrypted.requests().len(),
        2,
        "第一次先不等地同步一次，再等满五秒"
    );
    assert!(harness.matrix.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 凭口令创建时加密客户端没就绪就停用刚建的人物_令牌不交出去() {
    let harness = harness();
    *harness.encrypted.prepare_failure.lock().unwrap() = Some(NetworkGatewayFailure::Unavailable);

    assert_eq!(
        harness
            .gateway
            .create(create_with_code())
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert!(
        harness.agents.entered.lock().unwrap().is_empty(),
        "没进房间"
    );
    assert_eq!(*harness.agents.disabled.lock().unwrap(), [TOKEN]);
    assert_eq!(
        *harness.encrypted.forgotten.lock().unwrap(),
        [network_agent_id()]
    );
}

#[tokio::test(start_paused = true)]
async fn 再进一个房间_已经在里面原样返回_大厅直接进_私人房间先切到加密客户端() {
    let harness = harness();
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    let lobby = NetworkAgentRoom {
        catalog_id: RoomCatalogId::from_uuid(uuid(OTHER_AGENT)),
        matrix_room_id: MatrixRoomReference::new(ROOM.to_owned()).unwrap(),
        name: "Agent Room 大厅".to_owned(),
    };
    harness.agents.admissions.lock().unwrap().extend([
        Ok(NetworkAgentAdmission::AlreadyIn(lobby.clone())),
        Ok(NetworkAgentAdmission::Admitted(NetworkAgentTarget::Lobby(
            lobby.catalog_id,
        ))),
        Ok(NetworkAgentAdmission::Admitted(
            NetworkAgentTarget::Private(private_room()),
        )),
        Err(NetworkAgentFailure::new(
            NetworkAgentFailureKind::CodeInvalid,
        )),
    ]);

    let same = harness
        .gateway
        .enter_room(TOKEN, NetworkAgentRoomRequest::Lobby(None), [1; 32])
        .await
        .unwrap();
    assert_eq!(same, NetworkAgentEntry::Entered(lobby));
    assert!(
        harness.matrix.states.lock().unwrap().is_empty(),
        "已经在里面的不用再说"
    );
    let other = entered(
        harness
            .gateway
            .enter_room(
                TOKEN,
                NetworkAgentRoomRequest::Lobby(Some("rust-night".to_owned())),
                [1; 32],
            )
            .await
            .unwrap(),
    );
    assert_eq!(other.matrix_room_id.as_str(), SECOND_ROOM);
    assert!(
        harness.encrypted.prepared.lock().unwrap().is_empty(),
        "公开大厅不用加密客户端"
    );
    assert_online_announced(&harness);

    let private = entered(
        harness
            .gateway
            .enter_room(
                TOKEN,
                NetworkAgentRoomRequest::Code("K7P3-Q9XW-2DMA".to_owned()),
                [1; 32],
            )
            .await
            .unwrap(),
    );
    assert_eq!(private, private_room());
    assert_eq!(
        *harness.encrypted.prepared.lock().unwrap(),
        [Some(UtcMillis::new(42).unwrap())]
    );
    let log = harness.agents.log();
    assert_eq!(
        log[log.len() - 4..],
        ["mark_encrypted", "prepare", "enter", "refresh"]
    );
    assert_online_announced(&harness);

    assert_eq!(
        harness
            .gateway
            .enter_room(
                TOKEN,
                NetworkAgentRoomRequest::Code("0000-0000-0000".to_owned()),
                [1; 32]
            )
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::CodeInvalid
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn 拿房间号只敲门_不切加密客户端_也不说在线() {
    let harness = harness();
    let created = harness
        .gateway
        .create(CreateNetworkAgent {
            name: "Sol".to_owned(),
            room: NetworkAgentRoomRequest::Lobby(Some(private_room().catalog_id.to_string())),
            source_digest: [1; 32],
        })
        .await
        .expect("敲门成功");
    assert_eq!(created.placement, NetworkAgentPlacement::Knocked(knock()));

    harness
        .agents
        .admissions
        .lock()
        .unwrap()
        .push_back(Ok(NetworkAgentAdmission::Knocked(knock())));
    let again = harness
        .gateway
        .enter_room(
            TOKEN,
            NetworkAgentRoomRequest::Lobby(Some(private_room().catalog_id.to_string())),
            [1; 32],
        )
        .await
        .unwrap();
    assert_eq!(again, NetworkAgentEntry::Knocked(knock()));

    assert!(harness.encrypted.prepared.lock().unwrap().is_empty());
    assert!(harness.agents.entered.lock().unwrap().is_empty());
    assert!(
        harness.matrix.states.lock().unwrap().is_empty(),
        "不在房间里，不说在线"
    );
    assert!(harness.agents.disabled.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 放行后替它进房间_和凭口令一样先切到加密客户端建好身份_进了说在线() {
    let harness = harness();

    let room = harness
        .gateway
        .enter_admitted(agent(OWN_AGENT), private_room())
        .await
        .expect("替它进去了");

    assert_eq!(room, private_room());
    assert_eq!(
        harness.agents.log(),
        [
            "session_of_agent",
            "mark_encrypted",
            "prepare",
            "enter_admitted",
            "session_of_agent",
            "refresh",
        ],
        "身份建好之前不进；进了以后同步一次"
    );
    assert_eq!(
        *harness.agents.entered.lock().unwrap(),
        [NetworkAgentTarget::Private(private_room())]
    );
    assert_online_announced(&harness);

    // 之后它自己收消息也走加密客户端。
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    assert!(harness.matrix.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 放行时它已停用或加密客户端没就绪_回答失败_不进房间_也不停用它() {
    let harness = harness();
    assert_eq!(
        harness
            .gateway
            .enter_admitted(agent(OTHER_AGENT), private_room())
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::Unauthorized
        ))
    );
    assert!(harness.encrypted.prepared.lock().unwrap().is_empty());

    *harness.encrypted.prepare_failure.lock().unwrap() = Some(NetworkGatewayFailure::Unavailable);
    assert_eq!(
        harness
            .gateway
            .enter_admitted(agent(OWN_AGENT), private_room())
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert!(harness.agents.entered.lock().unwrap().is_empty());
    assert!(
        harness.agents.disabled.lock().unwrap().is_empty(),
        "放行没进成，敲门还在等，不停用它"
    );
}

#[tokio::test(start_paused = true)]
async fn 切到加密客户端时正在进行的长轮询立刻返回_免得轻量客户端跳过房间密钥() {
    let harness = Arc::new(harness());
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    let blocked = Arc::new(Notify::new());
    harness.matrix.push(Step::Block(blocked.clone()));
    let waiting = {
        let harness = harness.clone();
        tokio::spawn(async move {
            harness
                .gateway
                .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
                .await
        })
    };
    while harness.matrix.requests().len() < 2 {
        tokio::task::yield_now().await;
    }
    harness
        .agents
        .admissions
        .lock()
        .unwrap()
        .push_back(Ok(NetworkAgentAdmission::Admitted(
            NetworkAgentTarget::Private(private_room()),
        )));

    harness
        .gateway
        .enter_room(
            TOKEN,
            NetworkAgentRoomRequest::Code("K7P3-Q9XW-2DMA".to_owned()),
            [1; 32],
        )
        .await
        .unwrap();

    let returned = waiting.await.unwrap().unwrap();
    assert!(
        returned.messages.is_empty(),
        "旧的空手返回，轻量客户端的这次同步作废"
    );
    assert_eq!(harness.inbox.sync_token().as_deref(), Some("s1"));
    blocked.notify_waiters();
}

#[tokio::test(start_paused = true)]
async fn 进过加密房间的_agent_在加密房间里由加密客户端发言_正文先加密() {
    let harness = harness();
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());

    let sent = harness
        .gateway
        .send_message(TOKEN, draft("只说给房间里的人"))
        .await
        .expect("发出去了");

    assert!(sent.event.unwrap().starts_with("$encrypted-"));
    assert!(harness.matrix.sent().is_empty(), "不走轻量客户端");
    let client = &harness.encrypted.client;
    assert_eq!(
        *client.ensured.lock().unwrap(),
        1,
        "先确认身份就绪、刷新成员身份"
    );
    let events = client.sent.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0.as_str(), ROOM);
    assert!(
        events[0].1.content()["content"].get("encryption").is_some(),
        "正文的密钥随事件走，由客户端加密"
    );
    let uploads = harness.content.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        uploads[0].0.encryption_mode,
        ContentEncryptionMode::ClientE2ee
    );
    assert_ne!(uploads[0].1, "只说给房间里的人".as_bytes(), "存的是密文");
    assert_eq!(*harness.agents.quota_taken.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn 所有人只能在加密房间里发_公开大厅里拒绝且不扣次数() {
    let harness = harness();
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());
    let everyone = NetworkAgentMessageDraft {
        mentions_everyone: true,
        ..draft("大家看一下")
    };

    harness
        .gateway
        .send_message(TOKEN, everyone.clone())
        .await
        .expect("私人房间里能 @所有人");
    let events = harness.encrypted.client.sent.lock().unwrap().clone();
    assert_eq!(events[0].1.content()["preview"]["mentionsEveryone"], true);

    *harness.encrypted.client.encryption.lock().unwrap() = MatrixRoomEncryption::Unencrypted;
    assert_eq!(
        harness
            .gateway
            .send_message(TOKEN, everyone)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::InvalidMessage("mentionsEveryone")
    );
    assert_eq!(harness.encrypted.client.sent.lock().unwrap().len(), 1);
    assert_eq!(
        *harness.agents.quota_taken.lock().unwrap(),
        1,
        "拒绝的不扣次数"
    );
}

#[tokio::test(start_paused = true)]
async fn 加密客户端还不认识刚进的房间时完整同步一次再发() {
    let harness = harness();
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());
    harness
        .encrypted
        .client
        .readiness
        .lock()
        .unwrap()
        .push_back(Err(MatrixSecurityFailure::NotJoined));

    harness
        .gateway
        .send_message(TOKEN, draft("刚进来"))
        .await
        .expect("同步之后发出去了");

    assert_eq!(*harness.encrypted.refreshed.lock().unwrap(), 1);
    assert_eq!(*harness.encrypted.client.ensured.lock().unwrap(), 2);
    assert_eq!(harness.encrypted.client.sent.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn 进过加密房间的_agent_在公开大厅也由加密客户端发出_正文照常交给内容服务() {
    let harness = harness();
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());
    *harness.encrypted.client.encryption.lock().unwrap() = MatrixRoomEncryption::Unencrypted;

    harness
        .gateway
        .send_message(TOKEN, draft("大厅里的话"))
        .await
        .expect("发出去了");

    assert_eq!(
        *harness.encrypted.client.ensured.lock().unwrap(),
        0,
        "公开房间不用确认加密身份"
    );
    assert_eq!(harness.encrypted.client.sent.lock().unwrap().len(), 1);
    assert!(harness.matrix.sent().is_empty());
    let uploads = harness.content.uploads.lock().unwrap().clone();
    assert_eq!(
        uploads[0].0.encryption_mode,
        ContentEncryptionMode::ServerSide
    );
    assert_eq!(uploads[0].1, "大厅里的话".as_bytes());
}

#[tokio::test]
async fn 定时清理替停用的离开房间并记下_打不开的放弃_没离开成的下轮再试() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    // 服务器没开在线状态：房间里是它以前写的租约。
    harness
        .matrix
        .states
        .lock()
        .unwrap()
        .extend([ROOM, SECOND_ROOM].map(|room| (room.to_owned(), json!({ "status": "idle" }))));
    let lost = NetworkAgentId::from_uuid(Uuid::now_v7());
    *harness.agents.stale.lock().unwrap() = 2;
    *harness.agents.exits.lock().unwrap() = vec![
        NetworkAgentPendingExit::Session(Box::new(harness.agents.own_session())),
        NetworkAgentPendingExit::Unopenable(lost),
    ];
    *harness.matrix.leave_fails.lock().unwrap() = true;

    let first = harness.gateway.clean_up().await.unwrap();

    assert_eq!(
        first,
        NetworkAgentCleanupOutcome {
            disabled: 2,
            left: 0,
            abandoned: 1,
            retrying: 1,
            keys_deleted: 1,
            closed: 0,
        }
    );
    assert_eq!(*harness.agents.rooms_left.lock().unwrap(), [lost]);
    let states = harness.matrix.states.lock().unwrap().clone();
    assert!(
        states[2..]
            .iter()
            .all(|(_, state)| state["status"] == "offline")
            && states.len() == 4,
        "房间里是它的租约：离开前先在每个房间补“已离线”：{states:?}"
    );

    *harness.matrix.leave_fails.lock().unwrap() = false;
    let second = harness.gateway.clean_up().await.unwrap();

    assert_eq!(
        second,
        NetworkAgentCleanupOutcome {
            left: 1,
            keys_deleted: 1,
            ..NetworkAgentCleanupOutcome::default()
        }
    );
    assert_eq!(
        *harness.agents.rooms_left.lock().unwrap(),
        [lost, network_agent_id()]
    );
    assert_eq!(
        harness.gateway.clean_up().await.unwrap(),
        NetworkAgentCleanupOutcome::default()
    );
}

#[tokio::test]
async fn 离开房间以后先删加密存储再删钥匙_存储删不掉时钥匙留着下轮再删() {
    let harness = harness();
    *harness.agents.exits.lock().unwrap() = vec![NetworkAgentPendingExit::Session(Box::new(
        harness.agents.own_session(),
    ))];
    *harness.encrypted.remove_fails.lock().unwrap() = true;

    assert_eq!(
        harness.gateway.clean_up().await.unwrap(),
        NetworkAgentCleanupOutcome {
            left: 1,
            ..NetworkAgentCleanupOutcome::default()
        }
    );
    assert!(
        harness.agents.keys_deleted.lock().unwrap().is_empty(),
        "存储没删掉，钥匙留着"
    );

    *harness.encrypted.remove_fails.lock().unwrap() = false;
    assert_eq!(
        harness.gateway.clean_up().await.unwrap(),
        NetworkAgentCleanupOutcome {
            keys_deleted: 1,
            ..NetworkAgentCleanupOutcome::default()
        }
    );
    assert_eq!(
        *harness.encrypted.removed.lock().unwrap(),
        [network_agent_id()]
    );
    assert_eq!(
        *harness.agents.keys_deleted.lock().unwrap(),
        [network_agent_id()]
    );
    let steps: Vec<String> = harness
        .agents
        .log()
        .into_iter()
        .filter(|step| step == "remove_store" || step == "delete_keys")
        .collect();
    assert_eq!(
        steps,
        ["remove_store", "remove_store", "delete_keys"],
        "先删存储再删钥匙"
    );
    assert_eq!(
        harness.gateway.clean_up().await.unwrap(),
        NetworkAgentCleanupOutcome::default(),
        "删过的不再删"
    );
}

#[tokio::test]
async fn 没开加密客户端时_离开房间以后直接删钥匙() {
    let harness = build_harness(&[ROOM], false);
    *harness.agents.exits.lock().unwrap() = vec![NetworkAgentPendingExit::Session(Box::new(
        harness.agents.own_session(),
    ))];

    assert_eq!(
        harness.gateway.clean_up().await.unwrap(),
        NetworkAgentCleanupOutcome {
            left: 1,
            keys_deleted: 1,
            ..NetworkAgentCleanupOutcome::default()
        }
    );
    assert_eq!(
        *harness.agents.keys_deleted.lock().unwrap(),
        [network_agent_id()]
    );
    assert!(harness.encrypted.removed.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 自己发出去还不确定的_同步时按事务_id_对上() {
    let harness = harness();
    harness
        .matrix
        .send_failures
        .lock()
        .unwrap()
        .push_back(MatrixFailureKind::Timeout);
    let pending = harness
        .gateway
        .send_message(TOKEN, draft("到底发出去没有"))
        .await
        .unwrap();
    let (_, sent) = harness.matrix.sent().remove(0);
    let mut echoed = chat(
        "$echo:matrix.test",
        own(),
        pending.submission.as_uuid(),
        "到底发出去没有",
        [1; 64],
    );
    echoed = MatrixTimelineEvent::new(
        echoed.event_id().cloned(),
        echoed.sender().cloned(),
        echoed.event_type().clone(),
        None,
        Some(sent.transaction_id().clone()),
        echoed.origin_server_timestamp(),
        echoed.content().clone(),
    )
    .unwrap();
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", vec![echoed]))));

    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();

    assert_eq!(
        harness.submissions.state(pending.submission),
        Some(NetworkAgentSubmissionState::Accepted)
    );
}

// ---------- 在线状态 ----------

/// 进房间后先说的在线：会话里的每个房间都有一条“空闲”，不说在等消息。同样的状态发布服务只发一次，
/// 再进一个房间时只在新房间里说。
fn assert_online_announced(harness: &Harness) {
    let states = harness.matrix.states.lock().unwrap().clone();
    let rooms: std::collections::BTreeSet<String> = harness
        .agents
        .own_session()
        .rooms
        .iter()
        .map(|room| room.matrix_room_id.as_str().to_owned())
        .collect();
    let announced: std::collections::BTreeSet<String> =
        states.iter().map(|(room, _)| room.clone()).collect();
    assert_eq!(
        announced, rooms,
        "进房间后先在每个房间说一声在线：{states:?}"
    );
    for (_, state) in &states {
        assert_eq!(state["status"], "idle");
        assert_eq!(state["listeningUntil"], Value::Null, "进来时还没在等消息");
    }
}

#[tokio::test(start_paused = true)]
async fn 长轮询开始等待时在每个房间宣布一次_没再等十秒后清除_停用时先发离线() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 20))
        .await
        .unwrap();
    assert!(
        harness.matrix.states.lock().unwrap().is_empty(),
        "第一次同步不等，也就不说在等"
    );

    let started = tokio::time::Instant::now();
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 20))
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(30));

    // 每段最多等 10 秒；等待只在开始时宣布，每个房间一条，后两段不再重发。
    let requests = harness.matrix.requests();
    assert_eq!(
        requests[1..]
            .iter()
            .map(|request| request.timeout_millis)
            .collect::<Vec<_>>(),
        [10_000, 10_000, 10_000]
    );
    let states = harness.matrix.states.lock().unwrap().clone();
    assert_eq!(
        states
            .iter()
            .map(|(room, _)| room.as_str())
            .collect::<Vec<_>>(),
        [ROOM, SECOND_ROOM]
    );
    let first = &states[0].1;
    assert_eq!(first["status"], "idle");
    assert_eq!(first["actor"]["instanceId"], OWN_INSTANCE);
    assert_eq!(
        first["actor"]["agent"]["matrixUserId"],
        matrix_user(OWN_AGENT)
    );
    assert_eq!(first["listeningUntil"], "2025-09-23T04:00:15.000Z");
    assert_eq!(first["waitingUntil"], "2025-09-23T04:03:00.000Z");
    assert!(first["signature"].as_str().is_some());

    // 紧接着又等：上一次的看门狗看到它还在等，不清除。
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::from_secs(5), 20))
        .await
        .unwrap();
    assert_eq!(harness.matrix.states.lock().unwrap().len(), 2);

    // 之后不再等：十秒后在每个房间清除等待。
    tokio::time::sleep(WAIT_IDLE_TIMEOUT + Duration::from_secs(1)).await;
    let states = harness.matrix.states.lock().unwrap().clone();
    assert_eq!(states.len(), 4);
    for (_, cleared) in &states[2..] {
        assert_eq!(cleared["status"], "idle");
        assert_eq!(cleared["listeningUntil"], Value::Null);
        assert!(cleared.get("waitingUntil").is_none());
    }

    harness.gateway.leave_and_disable(TOKEN).await.unwrap();
    let states = harness.matrix.states.lock().unwrap().clone();
    let offline = &states[states.len() - 1].1;
    assert_eq!(offline["status"], "offline");
    assert_eq!(offline["listeningUntil"], Value::Null);
}

mod backfill;
mod before_join;
mod presence;
mod viewing;
