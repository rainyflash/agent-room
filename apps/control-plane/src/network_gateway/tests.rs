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
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentEncryptionSecrets,
        NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentLobby, NetworkAgentPendingExit,
        NetworkAgentResult, NetworkAgentSession, NetworkAgentUseCases, NetworkAgentView,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRecord,
        AgentInstanceVerificationRepository, Clock, ContentAccessMode, ContentAccessPolicy,
        DeviceSignature, MatrixAcceptedEvent, MatrixEvent, MatrixEventId, MatrixEventType,
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        MatrixRoomSync, MatrixRoomSyncKind, MatrixStateEvent, MatrixSyncBatch, MatrixSyncToken,
        MatrixTimelineEvent, MatrixTransactionId, MatrixUserId, NetworkAgentAckOutcome,
        NetworkAgentInboxAppend, NetworkAgentInboxAppendOutcome, NetworkAgentInboxChange,
        NetworkAgentInboxEntry, NetworkAgentInboxPage, NetworkAgentInboxStore,
        NetworkAgentMatrixGateway, NetworkAgentRoomRecord, NetworkAgentSubmissionClaim,
        NetworkAgentSubmissionClaimOutcome, NetworkAgentSubmissionRecord,
        NetworkAgentSubmissionState, NetworkAgentSubmissionStore, NetworkAgentSyncRequest,
        PortFuture, SecretValue,
    },
};
use agent_room_domain::{
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
use serde_json::{Value, json};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{
    EncryptedSessions, NetworkAgentCleanupOutcome, NetworkAgentMessageDraft, NetworkAgentMessaging,
    NetworkGateway, NetworkGatewayDependencies, NetworkGatewayFailure,
};

const TOKEN: &str = "network-agent-token";
const ROOM: &str = "!lobby:matrix.test";
const SECOND_ROOM: &str = "!second:matrix.test";
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
    /// 定时清理：停用了几个闲置的、待离开的有哪些、记为已离开的有哪些。
    stale: Mutex<usize>,
    exits: Mutex<Vec<NetworkAgentPendingExit>>,
    rooms_left: Mutex<Vec<NetworkAgentId>>,
    /// 进过加密房间的时刻；有值时网关改用加密客户端同步。
    encrypted_since: Mutex<Option<UtcMillis>>,
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
                })
                .collect(),
            quota: Mutex::new(None),
            quota_taken: Mutex::new(0),
            disabled: Mutex::new(Vec::new()),
            stale: Mutex::new(0),
            exits: Mutex::new(Vec::new()),
            rooms_left: Mutex::new(Vec::new()),
            encrypted_since: Mutex::new(None),
        }
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
    fn create(
        &self,
        _request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
        unreachable!("网关不创建网络 Agent")
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

    fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>> {
        unreachable!("网关不列大厅")
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
        scan_state: ContentScanState::Clean,
        lifecycle_state: ContentLifecycleState::Uploading,
        expires_at: None,
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

/// 与 Postgres 实现同样语义的收件箱：按到达编号，确认到哪条就删到哪条。
#[derive(Default)]
struct MemoryInbox {
    state: Mutex<InboxState>,
}

#[derive(Default)]
struct InboxState {
    sync_token: Option<MatrixSyncToken>,
    sequence: u64,
    dropped: u64,
    entries: Vec<(u64, MatrixEventId, MessageId, String, Value)>,
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
    fn pending(
        &self,
        _id: NetworkAgentId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentInboxPage>> {
        let state = self.state.lock().unwrap();
        let page = NetworkAgentInboxPage {
            sync_token: state.sync_token.clone(),
            entries: state
                .entries
                .iter()
                .take(usize::from(limit))
                .map(
                    |(sequence, event_id, _, _, preview)| NetworkAgentInboxEntry {
                        sequence: *sequence,
                        event_id: event_id.clone(),
                        preview: preview.clone(),
                    },
                )
                .collect(),
            pending: u64::try_from(state.entries.len()).unwrap(),
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
            for change in &append.changes {
                match change {
                    NetworkAgentInboxChange::Message(message) => {
                        if state
                            .entries
                            .iter()
                            .any(|(_, event_id, ..)| *event_id == message.event_id)
                        {
                            continue;
                        }
                        state.sequence += 1;
                        let sequence = state.sequence;
                        state.entries.push((
                            sequence,
                            message.event_id.clone(),
                            message.message_id,
                            message.actor_key.clone(),
                            message.preview.clone(),
                        ));
                        appended += 1;
                    }
                    NetworkAgentInboxChange::Replace {
                        message_id,
                        actor_key,
                        patch,
                        ..
                    } => {
                        for (_, _, id, actor, preview) in &mut state.entries {
                            if id == message_id && actor == actor_key {
                                for (key, value) in patch.as_object().unwrap() {
                                    preview[key] = value.clone();
                                }
                            }
                        }
                    }
                    NetworkAgentInboxChange::Redact {
                        message_id,
                        actor_key,
                        ..
                    } => state
                        .entries
                        .retain(|(_, _, id, actor, _)| !(id == message_id && actor == actor_key)),
                }
            }
            let capacity = usize::try_from(append.capacity).unwrap();
            if state.entries.len() > capacity {
                let excess = state.entries.len() - capacity;
                state.entries.drain(..excess);
                state.dropped += u64::try_from(excess).unwrap();
            }
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
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentAckOutcome>> {
        let mut state = self.state.lock().unwrap();
        let sequence = state
            .entries
            .iter()
            .find(|(_, id, ..)| id == event_id)
            .map(|(sequence, ..)| *sequence);
        let outcome = if let Some(sequence) = sequence {
            state.entries.retain(|(entry, ..)| *entry > sequence);
            state.dropped = 0;
            NetworkAgentAckOutcome::Acknowledged {
                pending: u64::try_from(state.entries.len()).unwrap(),
            }
        } else {
            NetworkAgentAckOutcome::NotPending {
                pending: u64::try_from(state.entries.len()).unwrap(),
            }
        };
        Box::pin(async move { Ok(outcome) })
    }
}

enum Step {
    Batch(MatrixResult<MatrixSyncBatch>),
    /// 一直等到被通知才返回空批次，模拟 Matrix 长轮询。
    Block(Arc<Notify>),
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
    left: Mutex<Vec<String>>,
    leave_fails: Mutex<bool>,
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
    forgotten: Mutex<Vec<NetworkAgentId>>,
    /// 下一轮清理时关掉几个闲置的。
    idle: Mutex<usize>,
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

    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()> {
        self.forgotten.lock().unwrap().push(id);
        Box::pin(async {})
    }

    fn evict_idle(&self) -> PortFuture<'_, usize> {
        let idle = std::mem::take(&mut *self.idle.lock().unwrap());
        Box::pin(async move { idle })
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
    let agents = Arc::new(FakeAgents::in_rooms(rooms));
    let inbox = Arc::new(MemoryInbox::default());
    let submissions = Arc::new(MemorySubmissions::default());
    let content = Arc::new(FakeContent::default());
    let matrix = Arc::new(ScriptedMatrix::default());
    let encrypted = Arc::new(FakeEncrypted::default());
    let gateway = NetworkGateway::new(NetworkGatewayDependencies {
        agents: agents.clone(),
        inbox: inbox.clone(),
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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

    // 没确认，再取还是这一条，不必再问 Matrix。
    let again = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .expect("再取");
    assert_eq!(texts(&again.messages), ["你好"]);
    assert_eq!(harness.matrix.requests().len(), 1);
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();
    assert_eq!(texts(&first.messages), ["一", "二"]);

    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$one:matrix.test")
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 1 }
    );
    let rest = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();
    assert_eq!(texts(&rest.messages), ["二"]);

    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$two:matrix.test")
            .await
            .unwrap(),
        NetworkAgentAckOutcome::Acknowledged { pending: 0 }
    );
    // 确认过的再确认一次不报错，只说明它不在收件箱里。
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "$two:matrix.test")
            .await
            .unwrap(),
        NetworkAgentAckOutcome::NotPending { pending: 0 }
    );

    let started = tokio::time::Instant::now();
    let empty = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(5), 20)
        .await
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    let requests = harness.matrix.requests();
    assert_eq!(requests[1].since.as_ref().unwrap().as_str(), "s1");
    assert_eq!(requests[1].timeout_millis, 5_000);
    assert_eq!(requests[1].timeline_limit, 50);
}

#[tokio::test(start_paused = true)]
async fn 只看一眼时有位置就不问_matrix() {
    let harness = harness();
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    assert_eq!(harness.matrix.requests().len(), 1, "第一次总要同步一次");

    let empty = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(harness.matrix.requests().len(), 1);
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    let blocked = Arc::new(Notify::new());
    harness.matrix.push(Step::Block(blocked.clone()));

    let old = {
        let harness = harness.clone();
        tokio::spawn(async move {
            harness
                .gateway
                .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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
            .wait_for_messages("wrong", Duration::from_secs(1), 20)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
            NetworkAgentFailureKind::Unauthorized
        ))
    );
    assert_eq!(
        harness
            .gateway
            .acknowledge(TOKEN, "not an event id")
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
            .wait_for_messages(TOKEN, Duration::from_secs(1), 20)
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert_eq!(harness.inbox.sync_token(), None, "失败的同步不推进位置");
}

#[tokio::test(start_paused = true)]
async fn 收件箱满了丢掉最早的并告诉_agent_丢了几条() {
    let harness = harness();
    let events = (0..205)
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 50)
        .await
        .unwrap();

    assert_eq!(received.messages.len(), 50);
    assert_eq!(received.pending, 200);
    assert_eq!(received.dropped, 5);
    assert_eq!(received.messages[0]["conversation"]["text"], "第 5 条");
}

// ---------- 发言 ----------

fn draft(text: &str) -> NetworkAgentMessageDraft {
    NetworkAgentMessageDraft {
        room_id: None,
        text: text.to_owned(),
        reply_to: None,
        mentions: Vec::new(),
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
                mentions: (0..9)
                    .map(|index| format!("@user{index}:matrix.test"))
                    .collect(),
                ..draft("太多提及")
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
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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
        .acknowledge(TOKEN, "$secret:matrix.test")
        .await
        .unwrap();
    let next = harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(5), 20)
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
            .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
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
            closed: 2,
            ..NetworkAgentCleanupOutcome::default()
        }
    );
    assert_eq!(
        *cleanup.encrypted.forgotten.lock().unwrap(),
        [network_agent_id(), lost]
    );
}

#[tokio::test]
async fn 定时清理替停用的离开房间并记下_打不开的放弃_没离开成的下轮再试() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
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
            closed: 0,
        }
    );
    assert_eq!(*harness.agents.rooms_left.lock().unwrap(), [lost]);
    let states = harness.matrix.states.lock().unwrap().clone();
    assert!(
        states.iter().all(|(_, state)| state["status"] == "offline") && states.len() == 2,
        "离开前先在每个房间发“已离线”：{states:?}"
    );

    *harness.matrix.leave_fails.lock().unwrap() = false;
    let second = harness.gateway.clean_up().await.unwrap();

    assert_eq!(
        second,
        NetworkAgentCleanupOutcome {
            left: 1,
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
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();

    assert_eq!(
        harness.submissions.state(pending.submission),
        Some(NetworkAgentSubmissionState::Accepted)
    );
}

// ---------- 在线状态 ----------

#[tokio::test(start_paused = true)]
async fn 长轮询期间在每个房间发等待消息_十五秒内续上_停用时先发离线() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s1", Vec::new()))));
    harness
        .gateway
        .wait_for_messages(TOKEN, Duration::ZERO, 20)
        .await
        .unwrap();
    assert!(
        harness.matrix.states.lock().unwrap().is_empty(),
        "第一次同步不等，也就不说在等"
    );

    let started = tokio::time::Instant::now();
    harness
        .gateway
        .wait_for_messages(TOKEN, Duration::from_secs(30), 20)
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(30));

    // 每段最多等 10 秒：三段各发一次，每次两个房间。
    let requests = harness.matrix.requests();
    assert_eq!(
        requests[1..]
            .iter()
            .map(|request| request.timeout_millis)
            .collect::<Vec<_>>(),
        [10_000, 10_000, 10_000]
    );
    let states = harness.matrix.states.lock().unwrap().clone();
    assert_eq!(states.len(), 6);
    assert_eq!(
        states
            .iter()
            .map(|(room, _)| room.as_str())
            .collect::<Vec<_>>(),
        [ROOM, SECOND_ROOM, ROOM, SECOND_ROOM, ROOM, SECOND_ROOM]
    );
    let first = &states[0].1;
    assert_eq!(first["status"], "idle");
    assert_eq!(first["actor"]["instanceId"], OWN_INSTANCE);
    assert_eq!(
        first["actor"]["agent"]["matrixUserId"],
        matrix_user(OWN_AGENT)
    );
    assert_eq!(first["listeningUntil"], "2025-09-23T04:00:15.000Z");
    assert!(first["signature"].as_str().is_some());
    assert_eq!(states[2].1["listeningUntil"], "2025-09-23T04:00:25.000Z");

    harness.gateway.leave_and_disable(TOKEN).await.unwrap();
    let states = harness.matrix.states.lock().unwrap().clone();
    let offline = &states[states.len() - 1].1;
    assert_eq!(offline["status"], "offline");
    assert_eq!(offline["listeningUntil"], Value::Null);
}
