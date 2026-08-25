use std::{collections::BTreeSet, sync::Arc};

use agent_room_application::ports::{
    Clock, DeviceSignature, MatrixEventId, MatrixRoomId, MatrixRoomStatePosition,
    MatrixRoomSyncKind, MatrixSyncBatch, MatrixTimelineEvent, MatrixUserId, PortFuture,
};
use agent_room_domain::{
    agent_status::{AgentTaskSummary, AgentWorkStatus},
    ids::{AgentId, AgentInstanceId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_protocol_conformance::generated::{
    AgentStatusEvent, AgentStatusVisibility as WireStatusVisibility,
    AgentWorkStatus as WireWorkStatus, Provenance,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use serde_json::Value;
use uuid::{Uuid, Version};

use crate::{
    agent_identity::BridgeAgentIdentity,
    agent_verification::{
        AgentEventAuthenticationDecision, AgentEventAuthenticationFailure,
        AgentEventAuthenticationFailureKind, AgentEventAuthenticator,
    },
};

pub const AGENT_STATUS_EVENT_TYPE: &str = "org.agentroom.agent.status.v1";
const ROOM_MEMBER_EVENT_TYPE: &str = "m.room.member";
const MAXIMUM_PRESENCE_TARGETS: usize = 50;
const MAXIMUM_STATUS_EVENTS_PER_ROOM: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceLeasePolicyError {
    InvalidClockSkew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceLeasePolicy {
    maximum_lease: DurationMillis,
    allowed_clock_skew: DurationMillis,
}

impl PresenceLeasePolicy {
    /// 创建接收端租约上限，禁止发送者用超长状态永久伪装在线。
    ///
    /// # Errors
    ///
    /// 容许偏差不小于最大租期时返回错误。
    pub fn new(
        maximum_lease: DurationMillis,
        allowed_clock_skew: DurationMillis,
    ) -> Result<Self, PresenceLeasePolicyError> {
        if allowed_clock_skew >= maximum_lease {
            return Err(PresenceLeasePolicyError::InvalidClockSkew);
        }
        Ok(Self {
            maximum_lease,
            allowed_clock_skew,
        })
    }

    pub const fn maximum_lease(self) -> DurationMillis {
        self.maximum_lease
    }

    pub const fn allowed_clock_skew(self) -> DurationMillis {
        self.allowed_clock_skew
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedAgentPresence {
    event_id: MatrixEventId,
    room_id: MatrixRoomId,
    identity: BridgeAgentIdentity,
    status: AgentWorkStatus,
    observed_at: UtcMillis,
    lease_expires_at: UtcMillis,
    origin_server_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedAgentPresenceFields {
    pub event_id: MatrixEventId,
    pub room_id: MatrixRoomId,
    pub identity: BridgeAgentIdentity,
    pub status: AgentWorkStatus,
    pub observed_at: UtcMillis,
    pub lease_expires_at: UtcMillis,
    pub origin_server_timestamp: u64,
}

impl ProjectedAgentPresence {
    /// 从已经完成协议校验与实例验签的字段恢复本机临时投影。
    pub fn from_verified_fields(fields: ProjectedAgentPresenceFields) -> Self {
        Self {
            event_id: fields.event_id,
            room_id: fields.room_id,
            identity: fields.identity,
            status: fields.status,
            observed_at: fields.observed_at,
            lease_expires_at: fields.lease_expires_at,
            origin_server_timestamp: fields.origin_server_timestamp,
        }
    }

    pub const fn event_id(&self) -> &MatrixEventId {
        &self.event_id
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn identity(&self) -> &BridgeAgentIdentity {
        &self.identity
    }

    pub const fn status(&self) -> AgentWorkStatus {
        self.status
    }

    pub const fn observed_at(&self) -> UtcMillis {
        self.observed_at
    }

    pub const fn lease_expires_at(&self) -> UtcMillis {
        self.lease_expires_at
    }

    pub const fn origin_server_timestamp(&self) -> u64 {
        self.origin_server_timestamp
    }

    #[must_use]
    fn revoked(mut self) -> Self {
        self.status = AgentWorkStatus::Offline;
        self.lease_expires_at = self.observed_at;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceObservation {
    presence: ProjectedAgentPresence,
    status: AgentWorkStatus,
    observed_at: UtcMillis,
}

impl PresenceObservation {
    pub const fn new(
        presence: ProjectedAgentPresence,
        status: AgentWorkStatus,
        observed_at: UtcMillis,
    ) -> Self {
        Self {
            presence,
            status,
            observed_at,
        }
    }

    pub const fn presence(&self) -> &ProjectedAgentPresence {
        &self.presence
    }

    pub const fn status(&self) -> AgentWorkStatus {
        self.status
    }

    pub const fn observed_at(&self) -> UtcMillis {
        self.observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceMembershipChange {
    matrix_user_id: MatrixUserId,
    joined: bool,
}

impl PresenceMembershipChange {
    pub const fn new(matrix_user_id: MatrixUserId, joined: bool) -> Self {
        Self {
            matrix_user_id,
            joined,
        }
    }

    pub const fn matrix_user_id(&self) -> &MatrixUserId {
        &self.matrix_user_id
    }

    pub const fn joined(&self) -> bool {
        self.joined
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceRoomProjectionMode {
    Delta,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceRoomProjection {
    room_id: MatrixRoomId,
    mode: PresenceRoomProjectionMode,
    memberships: Vec<PresenceMembershipChange>,
    presences: Vec<ProjectedAgentPresence>,
}

impl PresenceRoomProjection {
    pub const fn new(
        room_id: MatrixRoomId,
        mode: PresenceRoomProjectionMode,
        memberships: Vec<PresenceMembershipChange>,
        presences: Vec<ProjectedAgentPresence>,
    ) -> Self {
        Self {
            room_id,
            mode,
            memberships,
            presences,
        }
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn mode(&self) -> PresenceRoomProjectionMode {
        self.mode
    }

    pub fn memberships(&self) -> &[PresenceMembershipChange] {
        &self.memberships
    }

    pub fn presences(&self) -> &[ProjectedAgentPresence] {
        &self.presences
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceProjectionBatch {
    rooms: Vec<PresenceRoomProjection>,
}

impl PresenceProjectionBatch {
    pub const fn new(rooms: Vec<PresenceRoomProjection>) -> Self {
        Self { rooms }
    }

    pub fn rooms(&self) -> &[PresenceRoomProjection] {
        &self.rooms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceQuery {
    room_id: MatrixRoomId,
    agent_ids: BTreeSet<AgentId>,
    observed_at: UtcMillis,
}

impl PresenceQuery {
    /// 创建有界状态查询；空 Agent 集合表示查询当前房间全部实例。
    ///
    /// # Errors
    ///
    /// 显式目标超过协议上限时返回错误。
    pub fn new(
        room_id: MatrixRoomId,
        agent_ids: impl IntoIterator<Item = AgentId>,
        observed_at: UtcMillis,
    ) -> Result<Self, PresenceQueryError> {
        let agent_ids = agent_ids.into_iter().collect::<BTreeSet<_>>();
        if agent_ids.len() > MAXIMUM_PRESENCE_TARGETS {
            return Err(PresenceQueryError::TooManyTargets);
        }
        Ok(Self {
            room_id,
            agent_ids,
            observed_at,
        })
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn agent_ids(&self) -> &BTreeSet<AgentId> {
        &self.agent_ids
    }

    pub const fn observed_at(&self) -> UtcMillis {
        self.observed_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceQueryError {
    TooManyTargets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceProjectionFailureKind {
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceProjectionFailure {
    kind: PresenceProjectionFailureKind,
}

impl PresenceProjectionFailure {
    pub const fn new(kind: PresenceProjectionFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> PresenceProjectionFailureKind {
        self.kind
    }
}

pub trait PresenceProjectionRepository: Send + Sync {
    fn apply<'a>(
        &'a self,
        batch: &'a PresenceProjectionBatch,
    ) -> PortFuture<'a, Result<(), PresenceProjectionFailure>>;

    fn list<'a>(
        &'a self,
        query: &'a PresenceQuery,
    ) -> PortFuture<'a, Result<Vec<PresenceObservation>, PresenceProjectionFailure>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceSyncIssueReason {
    MissingEnvelope,
    InvalidEnvelope,
    SenderMismatch,
    InvalidMembership,
    TooManyStatusEvents,
    InvalidLease,
    FutureEvent,
    UnknownInstance,
    RevokedInstance,
    AgentInstanceMismatch,
    InvalidSignature,
    OutsideInstanceValidityWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSyncIssue {
    pub room_id: MatrixRoomId,
    pub event_id: Option<MatrixEventId>,
    pub reason: PresenceSyncIssueReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSyncOutcome {
    accepted_statuses: usize,
    membership_changes: usize,
    issues: Vec<PresenceSyncIssue>,
}

impl PresenceSyncOutcome {
    pub const fn accepted_statuses(&self) -> usize {
        self.accepted_statuses
    }

    pub const fn membership_changes(&self) -> usize {
        self.membership_changes
    }

    pub fn issues(&self) -> &[PresenceSyncIssue] {
        &self.issues
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceSyncFailureKind {
    Authentication,
    Projection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceSyncFailure {
    kind: PresenceSyncFailureKind,
    authentication: Option<AgentEventAuthenticationFailure>,
    projection: Option<PresenceProjectionFailure>,
}

impl PresenceSyncFailure {
    const fn authentication(failure: AgentEventAuthenticationFailure) -> Self {
        Self {
            kind: PresenceSyncFailureKind::Authentication,
            authentication: Some(failure),
            projection: None,
        }
    }

    const fn projection(failure: PresenceProjectionFailure) -> Self {
        Self {
            kind: PresenceSyncFailureKind::Projection,
            authentication: None,
            projection: Some(failure),
        }
    }

    pub const fn kind(self) -> PresenceSyncFailureKind {
        self.kind
    }

    pub const fn authentication_failure(self) -> Option<AgentEventAuthenticationFailure> {
        self.authentication
    }

    pub const fn projection_failure(self) -> Option<PresenceProjectionFailure> {
        self.projection
    }
}

pub struct PresenceSyncDependencies {
    pub authenticator: Arc<dyn AgentEventAuthenticator>,
    pub projections: Arc<dyn PresenceProjectionRepository>,
    pub clock: Arc<dyn Clock>,
}

pub struct PresenceSyncService {
    authenticator: Arc<dyn AgentEventAuthenticator>,
    projections: Arc<dyn PresenceProjectionRepository>,
    clock: Arc<dyn Clock>,
    policy: PresenceLeasePolicy,
}

impl PresenceSyncService {
    pub fn new(dependencies: PresenceSyncDependencies, policy: PresenceLeasePolicy) -> Self {
        Self {
            authenticator: dependencies.authenticator,
            projections: dependencies.projections,
            clock: dependencies.clock,
            policy,
        }
    }

    /// 从 Matrix 当前状态构建经实例签名验证的本机 Presence 投影。
    ///
    /// 单个畸形或不可信事件只被隔离；验签依赖或投影不可用时整个批次失败。
    ///
    /// # Errors
    ///
    /// 实例验签服务或本机投影不可用时返回阶段化错误。
    pub async fn process(
        &self,
        sync: &MatrixSyncBatch,
        full_state: bool,
    ) -> Result<PresenceSyncOutcome, PresenceSyncFailure> {
        let observed_at = self.clock.now();
        let mut room_updates = Vec::with_capacity(sync.rooms().len());
        let mut issues = Vec::new();
        let mut accepted_statuses = 0;
        let mut membership_changes = 0;

        for room in sync.rooms() {
            if room.kind() != MatrixRoomSyncKind::Joined {
                room_updates.push(PresenceRoomProjection::new(
                    room.room_id().clone(),
                    PresenceRoomProjectionMode::Remove,
                    Vec::new(),
                    Vec::new(),
                ));
                continue;
            }

            let events = presence_state_events(room);
            let mut memberships = Vec::new();
            let mut presences = Vec::new();
            let mut status_events = 0;
            for event in events {
                if event.event_type().as_str() == ROOM_MEMBER_EVENT_TYPE {
                    match parse_membership(event) {
                        Ok(change) => memberships.push(change),
                        Err(reason) => issues.push(issue(room.room_id(), event, reason)),
                    }
                    continue;
                }
                if event.event_type().as_str() != AGENT_STATUS_EVENT_TYPE {
                    continue;
                }
                status_events += 1;
                if status_events > MAXIMUM_STATUS_EVENTS_PER_ROOM {
                    issues.push(issue(
                        room.room_id(),
                        event,
                        PresenceSyncIssueReason::TooManyStatusEvents,
                    ));
                    continue;
                }
                let pending = match parse_status(room.room_id(), event, observed_at, self.policy) {
                    Ok(pending) => pending,
                    Err(reason) => {
                        issues.push(issue(room.room_id(), event, reason));
                        continue;
                    }
                };
                let decision = self
                    .authenticator
                    .authenticate(
                        pending.presence.identity.agent_id(),
                        pending.presence.identity.agent_instance_id(),
                        pending.origin_server_timestamp,
                        &pending.canonical_event,
                        &pending.signature,
                    )
                    .await
                    .map_err(PresenceSyncFailure::authentication)?;
                match decision {
                    AgentEventAuthenticationDecision::Trusted => {
                        presences.push(pending.presence);
                        accepted_statuses += 1;
                    }
                    AgentEventAuthenticationDecision::TrustedHistoricalRevoked => {
                        presences.push(pending.presence.revoked());
                        accepted_statuses += 1;
                    }
                    _ => issues.push(issue(room.room_id(), event, authentication_issue(decision))),
                }
            }
            membership_changes += memberships.len();
            room_updates.push(PresenceRoomProjection::new(
                room.room_id().clone(),
                if full_state {
                    PresenceRoomProjectionMode::Replace
                } else {
                    PresenceRoomProjectionMode::Delta
                },
                memberships,
                presences,
            ));
        }

        self.projections
            .apply(&PresenceProjectionBatch::new(room_updates))
            .await
            .map_err(PresenceSyncFailure::projection)?;
        Ok(PresenceSyncOutcome {
            accepted_statuses,
            membership_changes,
            issues,
        })
    }
}

struct PendingPresence {
    presence: ProjectedAgentPresence,
    origin_server_timestamp: UtcMillis,
    canonical_event: Vec<u8>,
    signature: DeviceSignature,
}

fn presence_state_events(
    room: &agent_room_application::ports::MatrixRoomSync,
) -> Vec<&MatrixTimelineEvent> {
    match room.state_position() {
        MatrixRoomStatePosition::BeforeTimeline => room
            .state()
            .iter()
            .chain(room.timeline())
            .filter(|event| event.state_key().is_some())
            .collect(),
        MatrixRoomStatePosition::AfterTimeline => room
            .state()
            .iter()
            .filter(|event| event.state_key().is_some())
            .collect(),
    }
}

fn parse_membership(
    event: &MatrixTimelineEvent,
) -> Result<PresenceMembershipChange, PresenceSyncIssueReason> {
    let state_key = event
        .state_key()
        .ok_or(PresenceSyncIssueReason::MissingEnvelope)?;
    let matrix_user_id = MatrixUserId::new(state_key.to_owned())
        .map_err(|_| PresenceSyncIssueReason::InvalidMembership)?;
    let membership = event
        .content()
        .get("membership")
        .and_then(Value::as_str)
        .ok_or(PresenceSyncIssueReason::InvalidMembership)?;
    if !matches!(membership, "ban" | "invite" | "join" | "knock" | "leave") {
        return Err(PresenceSyncIssueReason::InvalidMembership);
    }
    Ok(PresenceMembershipChange::new(
        matrix_user_id,
        membership == "join",
    ))
}

fn parse_status(
    room_id: &MatrixRoomId,
    event: &MatrixTimelineEvent,
    observed_at: UtcMillis,
    policy: PresenceLeasePolicy,
) -> Result<PendingPresence, PresenceSyncIssueReason> {
    let event_id = event
        .event_id()
        .cloned()
        .ok_or(PresenceSyncIssueReason::MissingEnvelope)?;
    let sender = event
        .sender()
        .ok_or(PresenceSyncIssueReason::MissingEnvelope)?;
    let state_key = event
        .state_key()
        .ok_or(PresenceSyncIssueReason::MissingEnvelope)?;
    let origin_server_timestamp = event
        .origin_server_timestamp()
        .and_then(|value| i64::try_from(value).ok())
        .and_then(|value| UtcMillis::new(value).ok())
        .ok_or(PresenceSyncIssueReason::MissingEnvelope)?;
    validate_property_limits(event.content())?;
    let (canonical_event, signature) = canonical_and_signature(event.content())?;
    let wire = serde_json::from_value::<AgentStatusEvent>(event.content().clone())
        .map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    validate_wire_status(&wire)?;
    if wire.schema_version != "1.0" || wire.event_type != AGENT_STATUS_EVENT_TYPE {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    let agent_id = AgentId::from_uuid(parse_v7(&wire.actor.agent.agent_id)?);
    let instance_id = AgentInstanceId::from_uuid(parse_v7(&wire.actor.instance_id)?);
    parse_v7(&wire.id)?;
    parse_v7(&wire.correlation_id)?;
    if state_key != wire.actor.instance_id || sender.as_str() != wire.actor.agent.matrix_user_id {
        return Err(PresenceSyncIssueReason::SenderMismatch);
    }
    if wire.actor.provenance != Provenance::AutonomousAgent {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    let mut identity = BridgeAgentIdentity::new(
        agent_id,
        wire.actor.agent.display_name,
        wire.actor.agent.matrix_user_id,
        instance_id,
    )
    .map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    if let Some(avatar_url) = wire.actor.agent.avatar_url {
        identity = identity
            .with_avatar_url(avatar_url)
            .map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    }
    let created_at = parse_time(&wire.created_at)?;
    let claimed_expiry = parse_time(&wire.lease_expires_at)?;
    let effective_expiry = evaluate_lease(created_at, claimed_expiry, observed_at, policy)?;
    let status = wire_status(&wire.status);
    Ok(PendingPresence {
        presence: ProjectedAgentPresence::from_verified_fields(ProjectedAgentPresenceFields {
            event_id,
            room_id: room_id.clone(),
            identity,
            status,
            observed_at,
            lease_expires_at: effective_expiry,
            origin_server_timestamp: event.origin_server_timestamp().expect("已检查服务端时间戳"),
        }),
        origin_server_timestamp,
        canonical_event,
        signature,
    })
}

fn validate_wire_status(wire: &AgentStatusEvent) -> Result<(), PresenceSyncIssueReason> {
    if wire.extensions.len() > 11
        || wire.actor.extensions.len() > 9
        || wire.actor.agent.extensions.len() > 12
    {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    let has_details =
        wire.task_summary.is_some() || wire.started_at.is_some() || wire.progress.is_some();
    if wire.visibility == WireStatusVisibility::Coarse && has_details {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    if let Some(summary) = wire.task_summary.as_ref() {
        AgentTaskSummary::new(summary.clone())
            .map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    }
    if let Some(started_at) = wire.started_at.as_deref() {
        parse_time(started_at)?;
    }
    if wire
        .progress
        .is_some_and(|progress| !progress.is_finite() || !(0.0..=1.0).contains(&progress))
    {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    Ok(())
}

fn evaluate_lease(
    created_at: UtcMillis,
    claimed_expiry: UtcMillis,
    observed_at: UtcMillis,
    policy: PresenceLeasePolicy,
) -> Result<UtcMillis, PresenceSyncIssueReason> {
    let maximum_lease = i64::try_from(policy.maximum_lease().value())
        .map_err(|_| PresenceSyncIssueReason::InvalidLease)?;
    let lifetime = claimed_expiry.value().saturating_sub(created_at.value());
    if lifetime <= 0 || lifetime > maximum_lease {
        return Err(PresenceSyncIssueReason::InvalidLease);
    }
    let latest_created_at = observed_at
        .checked_add(policy.allowed_clock_skew())
        .map_err(|_| PresenceSyncIssueReason::InvalidLease)?;
    if created_at > latest_created_at {
        return Err(PresenceSyncIssueReason::FutureEvent);
    }
    let sender_deadline = claimed_expiry
        .checked_add(policy.allowed_clock_skew())
        .map_err(|_| PresenceSyncIssueReason::InvalidLease)?;
    let local_deadline = observed_at
        .checked_add(policy.maximum_lease())
        .and_then(|value| value.checked_add(policy.allowed_clock_skew()))
        .map_err(|_| PresenceSyncIssueReason::InvalidLease)?;
    Ok(sender_deadline.min(local_deadline))
}

fn canonical_and_signature(
    content: &Value,
) -> Result<(Vec<u8>, DeviceSignature), PresenceSyncIssueReason> {
    let signature = content
        .get("signature")
        .and_then(Value::as_str)
        .ok_or(PresenceSyncIssueReason::InvalidEnvelope)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    let signature =
        DeviceSignature::new(bytes).map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    let mut unsigned = content.clone();
    unsigned
        .as_object_mut()
        .ok_or(PresenceSyncIssueReason::InvalidEnvelope)?
        .remove("signature")
        .ok_or(PresenceSyncIssueReason::InvalidEnvelope)?;
    let canonical =
        serde_jcs::to_vec(&unsigned).map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    Ok((canonical, signature))
}

fn validate_property_limits(content: &Value) -> Result<(), PresenceSyncIssueReason> {
    let object = content
        .as_object()
        .ok_or(PresenceSyncIssueReason::InvalidEnvelope)?;
    if object.len() > 24 {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    Ok(())
}

fn parse_v7(value: &str) -> Result<Uuid, PresenceSyncIssueReason> {
    let value = Uuid::parse_str(value).map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    if value.get_version() != Some(Version::SortRand) {
        return Err(PresenceSyncIssueReason::InvalidEnvelope);
    }
    Ok(value)
}

fn parse_time(value: &str) -> Result<UtcMillis, PresenceSyncIssueReason> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)?;
    UtcMillis::new(parsed.timestamp_millis()).map_err(|_| PresenceSyncIssueReason::InvalidEnvelope)
}

const fn wire_status(status: &WireWorkStatus) -> AgentWorkStatus {
    match status {
        WireWorkStatus::Offline => AgentWorkStatus::Offline,
        WireWorkStatus::Idle => AgentWorkStatus::Idle,
        WireWorkStatus::Working => AgentWorkStatus::Working,
        WireWorkStatus::WaitingInput => AgentWorkStatus::WaitingInput,
        WireWorkStatus::Blocked => AgentWorkStatus::Blocked,
        WireWorkStatus::Completed => AgentWorkStatus::Completed,
    }
}

const fn authentication_issue(
    decision: AgentEventAuthenticationDecision,
) -> PresenceSyncIssueReason {
    match decision {
        AgentEventAuthenticationDecision::UnknownInstance => {
            PresenceSyncIssueReason::UnknownInstance
        }
        AgentEventAuthenticationDecision::RevokedInstance => {
            PresenceSyncIssueReason::RevokedInstance
        }
        AgentEventAuthenticationDecision::AgentInstanceMismatch => {
            PresenceSyncIssueReason::AgentInstanceMismatch
        }
        AgentEventAuthenticationDecision::InvalidSignature => {
            PresenceSyncIssueReason::InvalidSignature
        }
        AgentEventAuthenticationDecision::OutsideInstanceValidityWindow => {
            PresenceSyncIssueReason::OutsideInstanceValidityWindow
        }
        AgentEventAuthenticationDecision::Trusted
        | AgentEventAuthenticationDecision::TrustedHistoricalRevoked => {
            PresenceSyncIssueReason::InvalidEnvelope
        }
    }
}

fn issue(
    room_id: &MatrixRoomId,
    event: &MatrixTimelineEvent,
    reason: PresenceSyncIssueReason,
) -> PresenceSyncIssue {
    PresenceSyncIssue {
        room_id: room_id.clone(),
        event_id: event.event_id().cloned(),
        reason,
    }
}

pub const fn reconnectable_presence_authentication(
    failure: AgentEventAuthenticationFailure,
) -> bool {
    matches!(
        failure.kind(),
        AgentEventAuthenticationFailureKind::Unavailable
    )
}
