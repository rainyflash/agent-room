//! 把本机 Bridge 的解析、验签和在线规则接到围观上：投影不落库，只截下这一次读到的结果。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use agent_room_application::ports::{
    AgentInstanceVerificationRecord, AgentInstanceVerificationRepository, Clock, MatrixEventId,
    MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
    MatrixTimelineEvent, MatrixTransactionId, PortFuture,
};
use agent_room_bridge_core::{
    agent_verification::{
        AgentEventAuthenticator, AgentInstanceVerificationGateway,
        AgentInstanceVerificationGatewayFailure, AgentInstanceVerificationGatewayFailureKind,
        AgentInstanceVerificationGatewayResult,
    },
    messages::{
        MessageProjectionMutation, MessageStoreFailure, MessageStoreFailureKind,
        MessageSubmissionClaim, MessageSubmissionClaimOutcome, MessageSubmissionRecord,
        MessageSubmissionRepository, MessageSyncDependencies, MessageSyncService,
    },
    presence::{
        AGENT_ROSTER_POLICY_EVENT_TYPE, AGENT_STATUS_EVENT_TYPE, PresenceLeasePolicy,
        PresenceObservation, PresenceProjectionBatch, PresenceProjectionFailure,
        PresenceProjectionRepository, PresenceQuery, PresenceSyncDependencies, PresenceSyncService,
    },
    presence_roster::project_roster,
};
use agent_room_domain::{
    agent_lifecycle::{AgentConnection, RECONNECT_GRACE_MS},
    ids::{AgentInstanceId, MessageSubmissionId},
    time::{DurationMillis, UtcMillis},
};
use serde_json::Value;

use crate::network_gateway::projection::CapturedProjection;

/// 在线状态的租约上限和容许的时钟偏差，与 Bridge 一样（`apps/bridge/src/runtime.rs`）。
const STATUS_LEASE_LIFETIME_MS: u64 = 300_000;
const STATUS_ALLOWED_CLOCK_SKEW_MS: u64 = 15_000;
/// 实例记录在内存里留这么久：这段时间里同一个实例只查一次库，撤销实例最多晚这么久生效。
const VERIFICATION_TTL_MS: i64 = 30_000;
const MAX_VERIFICATION_ENTRIES: usize = 4_096;
const MODERATION_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.moderation.notice.v1";
const ROOM_CREATE_EVENT_TYPE: &str = "m.room.create";
const ROOM_MEMBER_EVENT_TYPE: &str = "m.room.member";

/// 这次读到的东西没法验：查实例记录的库不可用，或者本机的投影接不上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadingFailure {
    Messages,
    Presence,
}

/// 用本机 Bridge 的同步服务解析、验签这一段消息，按时间先后截下结果。结构不对、验不过签的
/// 逐条隔离（不显示），不挡住别的。
pub(super) async fn verified_messages(
    authenticator: Arc<dyn AgentEventAuthenticator>,
    room_id: &MatrixRoomId,
    events: Vec<MatrixTimelineEvent>,
) -> Result<Vec<MessageProjectionMutation>, ReadingFailure> {
    let captured = Arc::new(CapturedProjection::default());
    let sync = MessageSyncService::new(MessageSyncDependencies {
        authenticator,
        projections: captured.clone(),
        submissions: Arc::new(NoSubmissions),
    });
    let room = MatrixRoomSync::new(
        room_id.clone(),
        MatrixRoomSyncKind::Joined,
        false,
        None,
        events,
        Vec::new(),
    );
    sync.process(&one_room(room).ok_or(ReadingFailure::Messages)?)
        .await
        .map_err(|_| ReadingFailure::Messages)?;
    Ok(captured
        .take()
        .map(|batch| batch.mutations().to_vec())
        .unwrap_or_default())
}

/// 此刻在大厅里、在线状态的租约还没过期（加 30 秒宽限）的 Agent，验过实例签名，按 Agent 合并。
pub(super) async fn online_agents(
    authenticator: Arc<dyn AgentEventAuthenticator>,
    clock: Arc<dyn Clock>,
    room_id: &MatrixRoomId,
    state: Vec<MatrixTimelineEvent>,
    now: UtcMillis,
) -> Result<Vec<PresenceObservation>, ReadingFailure> {
    let captured = Arc::new(CapturedPresence::default());
    let sync = PresenceSyncService::new(
        PresenceSyncDependencies {
            authenticator,
            projections: captured.clone(),
            clock,
        },
        lease_policy().ok_or(ReadingFailure::Presence)?,
    );
    let room = MatrixRoomSync::new(
        room_id.clone(),
        MatrixRoomSyncKind::Joined,
        false,
        None,
        Vec::new(),
        live_presence_state(state, now),
    );
    sync.process(&one_room(room).ok_or(ReadingFailure::Presence)?, true)
        .await
        .map_err(|_| ReadingFailure::Presence)?;
    let Some(batch) = captured.take() else {
        return Ok(Vec::new());
    };
    let Some(room) = batch.rooms().first() else {
        return Ok(Vec::new());
    };
    let joined: HashSet<&str> = room
        .memberships()
        .iter()
        .filter(|membership| membership.joined())
        .map(|membership| membership.matrix_user_id().as_str())
        .collect();
    let presences = room
        .presences()
        .iter()
        .filter(|presence| joined.contains(presence.identity().matrix_user_id().as_str()));
    Ok(
        project_roster(presences, now, room.policy().unwrap_or_default())
            .into_iter()
            .filter(|entry| entry.lifecycle.connection != AgentConnection::Offline)
            .collect(),
    )
}

/// 管理员隐藏的消息（事件 ID）。只认建大厅的账号写的：隐藏都是控制面以它的身份写的。
pub(super) fn hidden_events(state: &[MatrixTimelineEvent]) -> HashSet<String> {
    let Some(authority) = state
        .iter()
        .find(|event| {
            event.event_type().as_str() == ROOM_CREATE_EVENT_TYPE && event.state_key() == Some("")
        })
        .and_then(MatrixTimelineEvent::sender)
    else {
        return HashSet::new();
    };
    state
        .iter()
        .filter(|event| {
            event.event_type().as_str() == MODERATION_NOTICE_EVENT_TYPE
                && event.sender() == Some(authority)
                && event.content().get("hidden").and_then(Value::as_bool) == Some(true)
        })
        .filter_map(|event| {
            let target = event.state_key()?;
            (event.content().get("targetEventId").and_then(Value::as_str) == Some(target))
                .then(|| target.to_owned())
        })
        .collect()
}

/// 只留算在线要用的状态：在大厅里的成员、他们最近几分钟写过的在线状态、名单规则。
///
/// 进过大厅的每个实例都留着一条在线状态，日积月累；不先筛掉，每次都要验几千个签名。租约最长
/// 5 分钟，几分钟前写的在线状态加上宽限也早过期了。
fn live_presence_state(
    state: Vec<MatrixTimelineEvent>,
    now: UtcMillis,
) -> Vec<MatrixTimelineEvent> {
    let joined: HashSet<String> = state
        .iter()
        .filter(|event| is_joined_member(event))
        .filter_map(|event| event.state_key().map(str::to_owned))
        .collect();
    let lease_window = STATUS_LEASE_LIFETIME_MS + STATUS_ALLOWED_CLOCK_SKEW_MS;
    let oldest_live = now
        .value()
        .saturating_sub(i64::try_from(lease_window).unwrap_or(i64::MAX))
        .saturating_sub(RECONNECT_GRACE_MS);
    state
        .into_iter()
        .filter(|event| match event.event_type().as_str() {
            ROOM_MEMBER_EVENT_TYPE => is_joined_member(event),
            AGENT_STATUS_EVENT_TYPE => {
                event
                    .sender()
                    .is_some_and(|sender| joined.contains(sender.as_str()))
                    && event
                        .origin_server_timestamp()
                        .and_then(|timestamp| i64::try_from(timestamp).ok())
                        .is_some_and(|timestamp| timestamp >= oldest_live)
            }
            AGENT_ROSTER_POLICY_EVENT_TYPE => true,
            _ => false,
        })
        .collect()
}

fn is_joined_member(event: &MatrixTimelineEvent) -> bool {
    event.event_type().as_str() == ROOM_MEMBER_EVENT_TYPE
        && event.content().get("membership").and_then(Value::as_str) == Some("join")
}

fn lease_policy() -> Option<PresenceLeasePolicy> {
    PresenceLeasePolicy::new(
        DurationMillis::new(STATUS_LEASE_LIFETIME_MS).ok()?,
        DurationMillis::new(STATUS_ALLOWED_CLOCK_SKEW_MS).ok()?,
    )
    .ok()
}

fn one_room(room: MatrixRoomSync) -> Option<MatrixSyncBatch> {
    Some(MatrixSyncBatch::new(
        MatrixSyncToken::new("public-watch").ok()?,
        vec![room],
    ))
}

/// 只记下这一次处理的在线状态。
#[derive(Default)]
struct CapturedPresence {
    batch: Mutex<Option<PresenceProjectionBatch>>,
}

impl CapturedPresence {
    fn take(&self) -> Option<PresenceProjectionBatch> {
        self.batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl PresenceProjectionRepository for CapturedPresence {
    fn apply<'a>(
        &'a self,
        batch: &'a PresenceProjectionBatch,
    ) -> PortFuture<'a, Result<(), PresenceProjectionFailure>> {
        *self.batch.lock().unwrap_or_else(PoisonError::into_inner) = Some(batch.clone());
        Box::pin(async { Ok(()) })
    }

    fn list<'a>(
        &'a self,
        _query: &'a PresenceQuery,
    ) -> PortFuture<'a, Result<Vec<PresenceObservation>, PresenceProjectionFailure>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// 应用服务账号不发消息，没有要对账的提交。
struct NoSubmissions;

impl NoSubmissions {
    fn unused<T>() -> PortFuture<'static, Result<T, MessageStoreFailure>> {
        Box::pin(async {
            Err(MessageStoreFailure::new(
                MessageStoreFailureKind::Unavailable,
            ))
        })
    }
}

impl MessageSubmissionRepository for NoSubmissions {
    fn claim<'a>(
        &'a self,
        _claim: &'a MessageSubmissionClaim,
    ) -> PortFuture<'a, Result<MessageSubmissionClaimOutcome, MessageStoreFailure>> {
        Self::unused()
    }

    fn mark_submit_unknown(
        &self,
        _submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Self::unused()
    }

    fn mark_accepted<'a>(
        &'a self,
        _submission_id: MessageSubmissionId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Self::unused()
    }

    fn mark_bound(
        &self,
        _submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>> {
        Self::unused()
    }

    fn observe_transaction<'a>(
        &'a self,
        _transaction_id: &'a MatrixTransactionId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>> {
        Box::pin(async { Ok(None) })
    }
}

/// 验签要查的实例记录：同一个实例 30 秒内只查一次库。快照每 3 秒重读一次，大厅里的人来来去去
/// 就那么些。查不到的也记着，免得伪造的实例号每次都去查库；库不可用时不记，下次再查。
pub(super) struct CachedVerification {
    repository: Arc<dyn AgentInstanceVerificationRepository>,
    clock: Arc<dyn Clock>,
    entries: Mutex<HashMap<AgentInstanceId, (UtcMillis, Option<AgentInstanceVerificationRecord>)>>,
}

impl CachedVerification {
    pub(super) fn new(
        repository: Arc<dyn AgentInstanceVerificationRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            clock,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn entries(
        &self,
    ) -> MutexGuard<
        '_,
        HashMap<AgentInstanceId, (UtcMillis, Option<AgentInstanceVerificationRecord>)>,
    > {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn resolve_internal(
        &self,
        instance_id: AgentInstanceId,
    ) -> AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord> {
        let now = self.clock.now();
        let cached = self
            .entries()
            .get(&instance_id)
            .filter(|(fetched_at, _)| {
                now.value().saturating_sub(fetched_at.value()) < VERIFICATION_TTL_MS
            })
            .map(|(_, record)| record.clone());
        let record = if let Some(record) = cached {
            record
        } else {
            self.fetch(instance_id, now).await?
        };
        record.ok_or_else(|| {
            AgentInstanceVerificationGatewayFailure::new(
                AgentInstanceVerificationGatewayFailureKind::NotFound,
            )
        })
    }

    /// 查库并记下结果（查不到也记）；库不可用时不记。
    async fn fetch(
        &self,
        instance_id: AgentInstanceId,
        now: UtcMillis,
    ) -> AgentInstanceVerificationGatewayResult<Option<AgentInstanceVerificationRecord>> {
        let record = self
            .repository
            .find_verification_record(instance_id)
            .await
            .map_err(|_| {
                AgentInstanceVerificationGatewayFailure::new(
                    AgentInstanceVerificationGatewayFailureKind::Unavailable,
                )
            })?;
        let mut entries = self.entries();
        if entries.len() >= MAX_VERIFICATION_ENTRIES {
            entries.clear();
        }
        entries.insert(instance_id, (now, record.clone()));
        Ok(record)
    }
}

impl AgentInstanceVerificationGateway for CachedVerification {
    fn resolve(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>>
    {
        Box::pin(self.resolve_internal(instance_id))
    }
}
