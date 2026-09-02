use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    rooms::{RoomLanguage, RoomRegion},
};

use crate::{
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentLobbyAccessRepository, AgentRoomMembershipFactory, Clock, MatrixFailure,
        MatrixFailureKind, PortFuture, RoomAllocationEvidence, RoomAllocationMode,
        RoomAllocationStore,
    },
    rooms::{
        EnterLobbyDependencies, EnterLobbyFailure, EnterLobbyOutcome, EnterLobbyService,
        JoinLobbyDependencies, JoinLobbyFailure, JoinLobbyRequest, JoinLobbyService,
        LobbyJoinPolicy, LobbyJoinRollbackFailure, LobbyProvisioningFailure,
        LobbyProvisioningOperation, RoomReservationIdentifierFactory,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterAgentLobby {
    pub actor: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub catalog_id: RoomCatalogId,
    pub preferred_language: Option<RoomLanguage>,
    pub preferred_region: Option<RoomRegion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLobbyEntryFailureKind {
    Unauthorized,
    NotFound,
    Conflict,
    DependencyUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLobbyEntryFailure {
    Unauthorized,
    NotFound,
    Access(RepositoryError),
    Membership(MatrixFailure),
    Lobby(EnterLobbyFailure),
}

impl AgentLobbyEntryFailure {
    pub const fn kind(&self) -> AgentLobbyEntryFailureKind {
        match self {
            Self::Unauthorized => AgentLobbyEntryFailureKind::Unauthorized,
            Self::NotFound => AgentLobbyEntryFailureKind::NotFound,
            Self::Access(error) => repository_failure_kind(error.kind()),
            Self::Membership(error) => matrix_failure_kind(error.kind()),
            Self::Lobby(error) => lobby_failure_kind(error),
        }
    }
}

pub type AgentLobbyEntryResult<T> = Result<T, AgentLobbyEntryFailure>;

pub trait AgentLobbyEntryUseCases: Send + Sync {
    fn enter(
        &self,
        request: EnterAgentLobby,
    ) -> PortFuture<'_, AgentLobbyEntryResult<EnterLobbyOutcome>>;
}

pub struct AgentLobbyEntryDependencies {
    pub access: Arc<dyn AgentLobbyAccessRepository>,
    pub allocations: Arc<dyn RoomAllocationStore>,
    pub memberships: Arc<dyn AgentRoomMembershipFactory>,
    pub provisioning: Arc<dyn LobbyProvisioningOperation>,
    pub identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

/// 在设备、Agent 实例与 Matrix 身份三者严格绑定后执行现有大厅 Saga。
pub struct AgentLobbyEntryService {
    access: Arc<dyn AgentLobbyAccessRepository>,
    allocations: Arc<dyn RoomAllocationStore>,
    memberships: Arc<dyn AgentRoomMembershipFactory>,
    provisioning: Arc<dyn LobbyProvisioningOperation>,
    identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    clock: Arc<dyn Clock>,
    join_policy: LobbyJoinPolicy,
}

impl AgentLobbyEntryService {
    pub fn new(dependencies: AgentLobbyEntryDependencies, join_policy: LobbyJoinPolicy) -> Self {
        Self {
            access: dependencies.access,
            allocations: dependencies.allocations,
            memberships: dependencies.memberships,
            provisioning: dependencies.provisioning,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            join_policy,
        }
    }

    async fn enter_internal(
        &self,
        request: EnterAgentLobby,
    ) -> AgentLobbyEntryResult<EnterLobbyOutcome> {
        if request.actor.access_token_expires_at <= self.clock.now() {
            return Err(AgentLobbyEntryFailure::Unauthorized);
        }
        let access = self
            .access
            .find_lobby_access(request.agent_instance_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .ok_or(AgentLobbyEntryFailure::NotFound)?;
        if !access.active
            || access.agent_id != request.agent_id
            || access.agent_instance_id != request.agent_instance_id
            || access.device_id != request.actor.device_id
        {
            return Err(AgentLobbyEntryFailure::Unauthorized);
        }
        let membership = self
            .memberships
            .bind(&access.matrix_user_id)
            .map_err(AgentLobbyEntryFailure::Membership)?;
        let joins = Arc::new(JoinLobbyService::new(
            JoinLobbyDependencies {
                allocations: self.allocations.clone(),
                membership,
                identifiers: self.identifiers.clone(),
                clock: self.clock.clone(),
            },
            self.join_policy,
        ));
        EnterLobbyService::new(EnterLobbyDependencies {
            joins,
            provisioning: self.provisioning.clone(),
        })
        .enter(JoinLobbyRequest {
            agent_id: request.agent_id,
            agent_instance_id: request.agent_instance_id,
            catalog_id: request.catalog_id,
            mode: RoomAllocationMode::Automatic,
            preferred_language: request.preferred_language,
            preferred_region: request.preferred_region,
            evidence: RoomAllocationEvidence::default(),
        })
        .await
        .map_err(AgentLobbyEntryFailure::Lobby)
    }
}

impl AgentLobbyEntryUseCases for AgentLobbyEntryService {
    fn enter(
        &self,
        request: EnterAgentLobby,
    ) -> PortFuture<'_, AgentLobbyEntryResult<EnterLobbyOutcome>> {
        Box::pin(self.enter_internal(request))
    }
}

const fn repository_failure_kind(kind: RepositoryErrorKind) -> AgentLobbyEntryFailureKind {
    match kind {
        RepositoryErrorKind::Unavailable => AgentLobbyEntryFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden => AgentLobbyEntryFailureKind::Unauthorized,
        RepositoryErrorKind::NotFound => AgentLobbyEntryFailureKind::NotFound,
        RepositoryErrorKind::Conflict => AgentLobbyEntryFailureKind::Conflict,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            AgentLobbyEntryFailureKind::Internal
        }
    }
}

const fn matrix_failure_kind(kind: MatrixFailureKind) -> AgentLobbyEntryFailureKind {
    match kind {
        MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden => AgentLobbyEntryFailureKind::Unauthorized,
        MatrixFailureKind::NotFound => AgentLobbyEntryFailureKind::NotFound,
        MatrixFailureKind::Conflict => AgentLobbyEntryFailureKind::Conflict,
        MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::RateLimited => AgentLobbyEntryFailureKind::DependencyUnavailable,
        MatrixFailureKind::UnknownCommit => AgentLobbyEntryFailureKind::UnknownCommit,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => AgentLobbyEntryFailureKind::Internal,
    }
}

const fn lobby_failure_kind(failure: &EnterLobbyFailure) -> AgentLobbyEntryFailureKind {
    match failure {
        EnterLobbyFailure::Join(join) => join_failure_kind(join),
        EnterLobbyFailure::Provisioning(provisioning) => provisioning_failure_kind(provisioning),
        EnterLobbyFailure::ProvisioningResultMismatch { .. } => {
            AgentLobbyEntryFailureKind::Internal
        }
    }
}

const fn join_failure_kind(failure: &JoinLobbyFailure) -> AgentLobbyEntryFailureKind {
    match failure {
        JoinLobbyFailure::TimeOverflow => AgentLobbyEntryFailureKind::Internal,
        JoinLobbyFailure::Allocation(error) | JoinLobbyFailure::ConfirmationRolledBack(error) => {
            repository_failure_kind(error.kind())
        }
        JoinLobbyFailure::MatrixJoin(error) => matrix_failure_kind(error.kind()),
        JoinLobbyFailure::MatrixJoinCompensation { join, release } => combined_failure_kind(
            matrix_failure_kind(join.kind()),
            repository_failure_kind(release.kind()),
        ),
        JoinLobbyFailure::ConfirmationRollbackFailed {
            confirmation,
            rollback,
        } => combined_failure_kind(
            repository_failure_kind(confirmation.kind()),
            rollback_failure_kind(rollback),
        ),
    }
}

const fn rollback_failure_kind(failure: &LobbyJoinRollbackFailure) -> AgentLobbyEntryFailureKind {
    match failure {
        LobbyJoinRollbackFailure::Matrix(error) => matrix_failure_kind(error.kind()),
        LobbyJoinRollbackFailure::Reservation(error) => repository_failure_kind(error.kind()),
    }
}

const fn provisioning_failure_kind(
    failure: &LobbyProvisioningFailure,
) -> AgentLobbyEntryFailureKind {
    match failure {
        LobbyProvisioningFailure::Invalid(_) | LobbyProvisioningFailure::TimeOverflow => {
            AgentLobbyEntryFailureKind::Internal
        }
        LobbyProvisioningFailure::Store { source, .. } => repository_failure_kind(source.kind()),
        LobbyProvisioningFailure::Matrix { source, .. } => matrix_failure_kind(source.kind()),
        LobbyProvisioningFailure::MatrixReleaseFailed {
            source, release, ..
        } => combined_failure_kind(
            matrix_failure_kind(source.kind()),
            repository_failure_kind(release.kind()),
        ),
    }
}

const fn combined_failure_kind(
    primary: AgentLobbyEntryFailureKind,
    compensation: AgentLobbyEntryFailureKind,
) -> AgentLobbyEntryFailureKind {
    if matches!(primary, AgentLobbyEntryFailureKind::UnknownCommit)
        || matches!(compensation, AgentLobbyEntryFailureKind::UnknownCommit)
    {
        AgentLobbyEntryFailureKind::UnknownCommit
    } else if matches!(primary, AgentLobbyEntryFailureKind::DependencyUnavailable)
        || matches!(
            compensation,
            AgentLobbyEntryFailureKind::DependencyUnavailable
        )
    {
        AgentLobbyEntryFailureKind::DependencyUnavailable
    } else {
        AgentLobbyEntryFailureKind::Internal
    }
}
