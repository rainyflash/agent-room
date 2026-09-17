use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId},
    rooms::{MatrixRoomReference, RoomLanguage, RoomRegion},
};

use crate::{
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentLobbyAccessRecord, AgentLobbyAccessRepository, AgentRoomMembershipFactory, Clock,
        MatrixFailure, MatrixFailureKind, MatrixRoomId, PortFuture, PrivateMatrixMembership,
        PrivateRoomMatrixGateway, PrivateRoomStore, RoomAllocationEvidence, RoomAllocationMode,
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
    pub target_room: Option<MatrixRoomReference>,
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
    pub private_rooms: Arc<dyn PrivateRoomStore>,
    pub private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub memberships: Arc<dyn AgentRoomMembershipFactory>,
    pub provisioning: Arc<dyn LobbyProvisioningOperation>,
    pub identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

/// 在设备、Agent 实例与 Matrix 身份三者严格绑定后执行现有大厅 Saga。
pub struct AgentLobbyEntryService {
    access: Arc<dyn AgentLobbyAccessRepository>,
    allocations: Arc<dyn RoomAllocationStore>,
    private_rooms: Arc<dyn PrivateRoomStore>,
    private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
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
            private_rooms: dependencies.private_rooms,
            private_matrix: dependencies.private_matrix,
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
        let mode = if let Some(room) = &request.target_room {
            RoomAllocationMode::Manual(self.resolve_target(&request, &access, room).await?)
        } else {
            RoomAllocationMode::Automatic
        };
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
            mode,
            preferred_language: request.preferred_language,
            preferred_region: request.preferred_region,
            evidence: RoomAllocationEvidence::default(),
        })
        .await
        .map_err(AgentLobbyEntryFailure::Lobby)
    }

    /// 解析指名房间。公共大厅按目录可见性放行，其余交给私人房间的成员事实裁决。
    async fn resolve_target(
        &self,
        request: &EnterAgentLobby,
        access: &AgentLobbyAccessRecord,
        room: &MatrixRoomReference,
    ) -> AgentLobbyEntryResult<RoomInstanceId> {
        if let Some(target) = self
            .access
            .find_public_lobby_room(request.catalog_id, room)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
        {
            return Ok(target);
        }
        self.resolve_private_target(request, access, room).await
    }

    /// 私人房间不在公共目录里，Agent 随它此次代表的主体入场。
    ///
    /// 能力不超过该主体：只有已加入且可发言的成员才能带 Agent 进来，移除或封禁该成员会立即
    /// 让其 Agent 失去房间。目录必须与请求一致，避免用另一个房间的成员资格换取本房间的入场。
    async fn resolve_private_target(
        &self,
        request: &EnterAgentLobby,
        access: &AgentLobbyAccessRecord,
        room: &MatrixRoomReference,
    ) -> AgentLobbyEntryResult<RoomInstanceId> {
        let snapshot = self
            .private_rooms
            .find_by_matrix_room(room)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .ok_or(AgentLobbyEntryFailure::NotFound)?;
        if snapshot.catalog().id() != request.catalog_id {
            return Err(AgentLobbyEntryFailure::NotFound);
        }
        if !snapshot.room().admits_agent_of(access.principal_id) {
            return Err(AgentLobbyEntryFailure::Unauthorized);
        }
        let matrix_room =
            MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
                .map_err(|_| AgentLobbyEntryFailure::NotFound)?;
        // 私有 Matrix 房间只能受邀加入。重连是常态，已在房间时不再重复邀请。
        let membership = self
            .private_matrix
            .membership(&matrix_room, &access.matrix_user_id)
            .await
            .map_err(AgentLobbyEntryFailure::Membership)?;
        if !membership.is_some_and(PrivateMatrixMembership::is_joined) {
            self.private_matrix
                .invite(&matrix_room, &access.matrix_user_id)
                .await
                .map_err(AgentLobbyEntryFailure::Membership)?;
        }
        Ok(snapshot.instance().id())
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
