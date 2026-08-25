use std::{collections::BTreeMap, sync::Arc};

use agent_room_application::ports::{
    Clock, MatrixEventId, MatrixEventType, MatrixFailure, MatrixGateway, MatrixRoomId,
    MatrixStateEvent, MatrixStateKey,
};
use agent_room_domain::{
    DomainError,
    agent_status::{
        AgentStatusDetails, AgentStatusLease, AgentStatusSnapshot, AgentStatusVisibility,
        AgentWorkStatus,
    },
    time::{DurationMillis, UtcMillis},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{SecondsFormat, TimeZone as _, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::{Uuid, Version};

use crate::agent_identity::BridgeAgentIdentity;
use crate::ports::{
    AgentStatusStatePublisher, BridgeCredentialFailure, DeviceSigningIdentity,
    StatusEventIdentifierFactory,
};

const STATUS_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.agent.status.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAgentState {
    Disconnected,
    Available,
    Running,
    AwaitingInput,
    Blocked,
    Succeeded,
}

impl HostAgentState {
    pub const fn work_status(self) -> AgentWorkStatus {
        match self {
            Self::Disconnected => AgentWorkStatus::Offline,
            Self::Available => AgentWorkStatus::Idle,
            Self::Running => AgentWorkStatus::Working,
            Self::AwaitingInput => AgentWorkStatus::WaitingInput,
            Self::Blocked => AgentWorkStatus::Blocked,
            Self::Succeeded => AgentWorkStatus::Completed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusIntent {
    host_state: HostAgentState,
    details: Option<AgentStatusDetails>,
}

impl AgentStatusIntent {
    pub const fn new(host_state: HostAgentState, details: Option<AgentStatusDetails>) -> Self {
        Self {
            host_state,
            details,
        }
    }

    fn snapshot(
        &self,
        visibility: AgentStatusVisibility,
    ) -> Result<AgentStatusSnapshot, DomainError> {
        let details = match visibility {
            AgentStatusVisibility::Coarse => None,
            AgentStatusVisibility::Detailed => self.details.clone(),
        };
        AgentStatusSnapshot::new(self.host_state.work_status(), visibility, details)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusRoomTarget {
    room_id: MatrixRoomId,
    visibility: AgentStatusVisibility,
}

impl AgentStatusRoomTarget {
    pub const fn new(room_id: MatrixRoomId, visibility: AgentStatusVisibility) -> Self {
        Self {
            room_id,
            visibility,
        }
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn visibility(&self) -> AgentStatusVisibility {
        self.visibility
    }
}

pub type AgentStatusIdentity = BridgeAgentIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStatusLeasePolicy {
    lifetime: DurationMillis,
    renewal_interval: DurationMillis,
    renewal_jitter: DurationMillis,
}

impl AgentStatusLeasePolicy {
    /// 创建状态租约及续租抖动策略。
    ///
    /// # Errors
    ///
    /// 抖动不小于基础间隔，或最晚续租时间不早于租约到期时返回错误。
    pub fn new(
        lifetime: DurationMillis,
        renewal_interval: DurationMillis,
        renewal_jitter: DurationMillis,
    ) -> Result<Self, StatusPublicationFailure> {
        let latest_renewal = renewal_interval
            .value()
            .checked_add(renewal_jitter.value())
            .ok_or_else(|| {
                StatusPublicationFailure::new(StatusPublicationFailureKind::InvalidConfiguration)
            })?;
        if renewal_jitter >= renewal_interval || latest_renewal >= lifetime.value() {
            return Err(StatusPublicationFailure::new(
                StatusPublicationFailureKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            lifetime,
            renewal_interval,
            renewal_jitter,
        })
    }

    fn renewal_delay(self, entropy: u64) -> DurationMillis {
        let minimum = self.renewal_interval.value() - self.renewal_jitter.value();
        let width = self
            .renewal_jitter
            .value()
            .saturating_mul(2)
            .saturating_add(1);
        let delay = minimum.saturating_add(entropy % width);
        DurationMillis::new(delay).expect("续租策略保证延迟非零")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPublicationReason {
    Initial,
    StatusChanged,
    VisibilityChanged,
    Renewal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusPublicationOutcome {
    Published {
        event_id: MatrixEventId,
        reason: StatusPublicationReason,
        lease: AgentStatusLease,
        renew_at: UtcMillis,
    },
    NotDue {
        renew_at: UtcMillis,
        lease_expires_at: UtcMillis,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPublicationFailureKind {
    InvalidConfiguration,
    InvalidIdentity,
    InvalidIntent,
    InvalidIdentifier,
    SigningUnavailable,
    Serialization,
    Matrix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusPublicationFailure {
    kind: StatusPublicationFailureKind,
    matrix_failure: Option<MatrixFailure>,
}

impl StatusPublicationFailure {
    const fn new(kind: StatusPublicationFailureKind) -> Self {
        Self {
            kind,
            matrix_failure: None,
        }
    }

    const fn matrix(failure: MatrixFailure) -> Self {
        Self {
            kind: StatusPublicationFailureKind::Matrix,
            matrix_failure: Some(failure),
        }
    }

    pub const fn kind(self) -> StatusPublicationFailureKind {
        self.kind
    }

    pub const fn matrix_failure(self) -> Option<MatrixFailure> {
        self.matrix_failure
    }
}

pub type StatusPublicationResult<T> = Result<T, StatusPublicationFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishedRoomStatus {
    snapshot: AgentStatusSnapshot,
    renew_at: UtcMillis,
    lease_expires_at: UtcMillis,
}

pub struct AgentStatusPublicationService {
    identity: AgentStatusIdentity,
    signer: Arc<dyn DeviceSigningIdentity>,
    publisher: Arc<dyn AgentStatusStatePublisher>,
    identifiers: Arc<dyn StatusEventIdentifierFactory>,
    clock: Arc<dyn Clock>,
    policy: AgentStatusLeasePolicy,
    published_rooms: BTreeMap<MatrixRoomId, PublishedRoomStatus>,
}

pub struct AgentStatusPublicationDependencies {
    pub identity: AgentStatusIdentity,
    pub signer: Arc<dyn DeviceSigningIdentity>,
    pub publisher: Arc<dyn AgentStatusStatePublisher>,
    pub identifiers: Arc<dyn StatusEventIdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl AgentStatusPublicationService {
    pub fn new(
        dependencies: AgentStatusPublicationDependencies,
        policy: AgentStatusLeasePolicy,
    ) -> Self {
        Self {
            identity: dependencies.identity,
            signer: dependencies.signer,
            publisher: dependencies.publisher,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            policy,
            published_rooms: BTreeMap::new(),
        }
    }

    /// 在首次发布、状态变化、可见性变化或续租到期时写入 Matrix 房间状态。
    ///
    /// # Errors
    ///
    /// 意图、标识、签名、序列化或 Matrix 发布失败时返回稳定错误；失败不会推进续租游标。
    pub async fn publish_if_due(
        &mut self,
        target: &AgentStatusRoomTarget,
        intent: &AgentStatusIntent,
        entropy: u64,
    ) -> StatusPublicationResult<StatusPublicationOutcome> {
        let now = self.clock.now();
        let snapshot = intent.snapshot(target.visibility()).map_err(|_| {
            StatusPublicationFailure::new(StatusPublicationFailureKind::InvalidIntent)
        })?;
        let reason = if let Some(previous) = self.published_rooms.get(target.room_id()) {
            if previous.snapshot.status() != snapshot.status() {
                StatusPublicationReason::StatusChanged
            } else if previous.snapshot.visibility() != snapshot.visibility() {
                StatusPublicationReason::VisibilityChanged
            } else if now >= previous.renew_at {
                StatusPublicationReason::Renewal
            } else {
                return Ok(StatusPublicationOutcome::NotDue {
                    renew_at: previous.renew_at,
                    lease_expires_at: previous.lease_expires_at,
                });
            }
        } else {
            StatusPublicationReason::Initial
        };
        let lease = AgentStatusLease::issue(
            self.identity.agent_instance_id(),
            snapshot.clone(),
            now,
            self.policy.lifetime,
        )
        .map_err(|_| StatusPublicationFailure::new(StatusPublicationFailureKind::InvalidIntent))?;
        let event = self.state_event(&lease)?;
        let event_id = self
            .publisher
            .publish(target.room_id(), &event)
            .await
            .map_err(StatusPublicationFailure::matrix)?;
        let renew_at = now
            .checked_add(self.policy.renewal_delay(entropy))
            .map_err(|_| {
                StatusPublicationFailure::new(StatusPublicationFailureKind::InvalidConfiguration)
            })?;
        self.published_rooms.insert(
            target.room_id().clone(),
            PublishedRoomStatus {
                snapshot,
                renew_at,
                lease_expires_at: lease.expires_at(),
            },
        );
        Ok(StatusPublicationOutcome::Published {
            event_id,
            reason,
            lease,
            renew_at,
        })
    }

    fn state_event(&self, lease: &AgentStatusLease) -> StatusPublicationResult<MatrixStateEvent> {
        let event_id = self.identifiers.event_id();
        let correlation_id = self.identifiers.correlation_id();
        if event_id.get_version() != Some(Version::SortRand)
            || correlation_id.get_version() != Some(Version::SortRand)
        {
            return Err(StatusPublicationFailure::new(
                StatusPublicationFailureKind::InvalidIdentifier,
            ));
        }
        let unsigned = UnsignedStatusEvent::new(&self.identity, lease, event_id, correlation_id)?;
        let mut content = serde_json::to_value(unsigned).map_err(|_| {
            StatusPublicationFailure::new(StatusPublicationFailureKind::Serialization)
        })?;
        let canonical = serde_jcs::to_vec(&content).map_err(|_| {
            StatusPublicationFailure::new(StatusPublicationFailureKind::Serialization)
        })?;
        let signature = self.signer.sign(&canonical).map_err(map_signing_failure)?;
        let Some(content_object) = content.as_object_mut() else {
            return Err(StatusPublicationFailure::new(
                StatusPublicationFailureKind::Serialization,
            ));
        };
        content_object.insert(
            "signature".to_owned(),
            Value::String(URL_SAFE_NO_PAD.encode(signature.as_bytes())),
        );
        MatrixStateEvent::new(
            MatrixEventType::new(STATUS_EVENT_TYPE).map_err(|_| {
                StatusPublicationFailure::new(StatusPublicationFailureKind::Serialization)
            })?,
            MatrixStateKey::from_agent_instance_id(self.identity.agent_instance_id()),
            content,
        )
        .map_err(|_| StatusPublicationFailure::new(StatusPublicationFailureKind::Serialization))
    }
}

pub struct MatrixStatusStatePublisher {
    gateway: Arc<dyn MatrixGateway>,
}

impl MatrixStatusStatePublisher {
    pub fn new(gateway: Arc<dyn MatrixGateway>) -> Self {
        Self { gateway }
    }
}

impl AgentStatusStatePublisher for MatrixStatusStatePublisher {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> agent_room_application::ports::PortFuture<
        'a,
        agent_room_application::ports::MatrixResult<MatrixEventId>,
    > {
        self.gateway.send_state_event(room_id, event)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedStatusEvent<'a> {
    schema_version: &'static str,
    event_type: &'static str,
    id: Uuid,
    created_at: String,
    actor: StatusActor<'a>,
    correlation_id: Uuid,
    status: &'static str,
    visibility: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_summary: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<f64>,
    lease_expires_at: String,
}

impl<'a> UnsignedStatusEvent<'a> {
    fn new(
        identity: &'a AgentStatusIdentity,
        lease: &'a AgentStatusLease,
        event_id: Uuid,
        correlation_id: Uuid,
    ) -> StatusPublicationResult<Self> {
        let details = lease.snapshot().details();
        Ok(Self {
            schema_version: "1.0",
            event_type: STATUS_EVENT_TYPE,
            id: event_id,
            created_at: rfc3339(lease.published_at())?,
            actor: StatusActor {
                agent: StatusAgent {
                    agent_id: identity.agent_id().as_uuid(),
                    display_name: identity.display_name(),
                    matrix_user_id: identity.matrix_user_id().as_str(),
                },
                instance_id: identity.agent_instance_id().as_uuid(),
                provenance: "autonomous_agent",
            },
            correlation_id,
            status: lease.snapshot().status().as_str(),
            visibility: lease.snapshot().visibility().as_str(),
            task_summary: details
                .and_then(AgentStatusDetails::task_summary)
                .map(agent_room_domain::agent_status::AgentTaskSummary::as_str),
            started_at: details
                .and_then(AgentStatusDetails::started_at)
                .map(rfc3339)
                .transpose()?,
            progress: details
                .and_then(AgentStatusDetails::progress)
                .map(agent_room_domain::agent_status::AgentTaskProgress::fraction),
            lease_expires_at: rfc3339(lease.expires_at())?,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusActor<'a> {
    agent: StatusAgent<'a>,
    instance_id: Uuid,
    provenance: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusAgent<'a> {
    agent_id: Uuid,
    display_name: &'a str,
    matrix_user_id: &'a str,
}

fn rfc3339(value: UtcMillis) -> StatusPublicationResult<String> {
    Utc.timestamp_millis_opt(value.value())
        .single()
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| StatusPublicationFailure::new(StatusPublicationFailureKind::Serialization))
}

const fn map_signing_failure(_failure: BridgeCredentialFailure) -> StatusPublicationFailure {
    StatusPublicationFailure::new(StatusPublicationFailureKind::SigningUnavailable)
}
