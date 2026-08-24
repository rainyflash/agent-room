use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    agent_cards::{
        AgentCardSnapshot, AgentCardSnapshotFields, AgentCardSourceUrl, NormalizedAgentCard,
    },
    agents::AgentStatus,
    ids::AgentId,
};

use crate::{
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentCardFetchFailureKind, AgentCardSnapshotRepository, AgentCardSource,
        AgentMembershipRepository, AgentRepository, Clock, IdentifierFactory, PortFuture,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshAgentCard {
    pub actor: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub source_url: AgentCardSourceUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardChange {
    Initial,
    Unchanged,
    ProfileChanged,
    CapabilitySurfaceChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardRefresh {
    pub snapshot: AgentCardSnapshot,
    pub change: AgentCardChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardManagementFailureKind {
    InvalidRequest,
    Forbidden,
    NotFound,
    UntrustedSource,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentCardManagementFailure {
    operation: &'static str,
    kind: AgentCardManagementFailureKind,
}

impl AgentCardManagementFailure {
    const fn new(operation: &'static str, kind: AgentCardManagementFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentCardManagementFailureKind {
        self.kind
    }
}

pub type AgentCardManagementResult<T> = Result<T, AgentCardManagementFailure>;

pub trait AgentCardUseCases: Send + Sync {
    fn refresh(
        &self,
        request: RefreshAgentCard,
    ) -> PortFuture<'_, AgentCardManagementResult<AgentCardRefresh>>;
}

pub struct AgentCardService {
    agents: Arc<dyn AgentRepository>,
    memberships: Arc<dyn AgentMembershipRepository>,
    source: Arc<dyn AgentCardSource>,
    snapshots: Arc<dyn AgentCardSnapshotRepository>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
}

pub struct AgentCardDependencies {
    pub agents: Arc<dyn AgentRepository>,
    pub memberships: Arc<dyn AgentMembershipRepository>,
    pub source: Arc<dyn AgentCardSource>,
    pub snapshots: Arc<dyn AgentCardSnapshotRepository>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl AgentCardService {
    pub fn new(dependencies: AgentCardDependencies) -> Self {
        Self {
            agents: dependencies.agents,
            memberships: dependencies.memberships,
            source: dependencies.source,
            snapshots: dependencies.snapshots,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
        }
    }

    async fn refresh_internal(
        &self,
        request: RefreshAgentCard,
    ) -> AgentCardManagementResult<AgentCardRefresh> {
        let operation = "agent_card.refresh";
        let now = self.clock.now();
        ensure_active_device(&request.actor, now, operation)?;
        let memberships = self
            .memberships
            .find_memberships(request.agent_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, AgentCardManagementFailureKind::NotFound))?;
        memberships
            .ensure_can_register_instance(request.actor.account.principal.id())
            .map_err(|error| map_domain_failure(operation, &error))?;
        let agent = self
            .agents
            .find(request.agent_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, AgentCardManagementFailureKind::NotFound))?;
        if agent.status() != AgentStatus::Active {
            return Err(failure(
                operation,
                AgentCardManagementFailureKind::Forbidden,
            ));
        }

        let fetched = self
            .source
            .fetch(&request.source_url)
            .await
            .map_err(|error| map_fetch_failure(operation, error.kind()))?;
        let expires_at = now
            .checked_add(fetched.cache_lifetime)
            .map_err(|error| map_domain_failure(operation, &error))?;
        let latest = self
            .snapshots
            .find_latest(request.agent_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        let snapshot = AgentCardSnapshot::new(AgentCardSnapshotFields {
            id: self.identifiers.agent_card_snapshot_id(),
            agent_id: request.agent_id,
            source_url: request.source_url,
            digest: fetched.digest,
            card: fetched.card,
            verification: fetched.verification,
            fetched_at: now,
            expires_at,
        })
        .map_err(|error| map_domain_failure(operation, &error))?;
        let change = classify_change(latest.as_ref(), &snapshot);
        let snapshot = self
            .snapshots
            .save(&snapshot)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        Ok(AgentCardRefresh { snapshot, change })
    }
}

impl AgentCardUseCases for AgentCardService {
    fn refresh(
        &self,
        request: RefreshAgentCard,
    ) -> PortFuture<'_, AgentCardManagementResult<AgentCardRefresh>> {
        Box::pin(async move { self.refresh_internal(request).await })
    }
}

fn classify_change(
    latest: Option<&AgentCardSnapshot>,
    current: &AgentCardSnapshot,
) -> AgentCardChange {
    let Some(latest) = latest else {
        return AgentCardChange::Initial;
    };
    if latest.digest() == current.digest() {
        return AgentCardChange::Unchanged;
    }
    if same_capability_surface(latest.card(), current.card()) {
        AgentCardChange::ProfileChanged
    } else {
        AgentCardChange::CapabilitySurfaceChanged
    }
}

fn same_capability_surface(left: &NormalizedAgentCard, right: &NormalizedAgentCard) -> bool {
    left.endpoints() == right.endpoints()
        && left.capabilities() == right.capabilities()
        && left.security_schemes() == right.security_schemes()
        && left.default_input_modes() == right.default_input_modes()
        && left.default_output_modes() == right.default_output_modes()
        && left.skills() == right.skills()
}

fn ensure_active_device(
    actor: &AuthenticatedDevice,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> AgentCardManagementResult<()> {
    if actor.account.principal.allows_authentication() && now < actor.access_token_expires_at {
        Ok(())
    } else {
        Err(failure(
            operation,
            AgentCardManagementFailureKind::Forbidden,
        ))
    }
}

fn map_domain_failure(operation: &'static str, error: &DomainError) -> AgentCardManagementFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => AgentCardManagementFailureKind::Forbidden,
        DomainError::Validation { .. }
        | DomainError::InvalidTransition { .. }
        | DomainError::CapacityExceeded { .. } => AgentCardManagementFailureKind::InvalidRequest,
        DomainError::InvariantViolation { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => AgentCardManagementFailureKind::Internal,
    };
    failure(operation, kind)
}

fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> AgentCardManagementFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Forbidden => AgentCardManagementFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => AgentCardManagementFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => AgentCardManagementFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Conflict | RepositoryErrorKind::Constraint => {
            AgentCardManagementFailureKind::InvalidRequest
        }
        RepositoryErrorKind::CorruptData => AgentCardManagementFailureKind::Internal,
    };
    failure(operation, kind)
}

fn map_fetch_failure(
    operation: &'static str,
    kind: AgentCardFetchFailureKind,
) -> AgentCardManagementFailure {
    let kind = match kind {
        AgentCardFetchFailureKind::RejectedSource
        | AgentCardFetchFailureKind::InvalidResponse
        | AgentCardFetchFailureKind::UnsupportedProtocol => {
            AgentCardManagementFailureKind::InvalidRequest
        }
        AgentCardFetchFailureKind::BlockedNetworkTarget
        | AgentCardFetchFailureKind::InvalidSignature => {
            AgentCardManagementFailureKind::UntrustedSource
        }
        AgentCardFetchFailureKind::Unavailable => {
            AgentCardManagementFailureKind::DependencyUnavailable
        }
        AgentCardFetchFailureKind::Internal => AgentCardManagementFailureKind::Internal,
    };
    failure(operation, kind)
}

const fn failure(
    operation: &'static str,
    kind: AgentCardManagementFailureKind,
) -> AgentCardManagementFailure {
    AgentCardManagementFailure::new(operation, kind)
}
