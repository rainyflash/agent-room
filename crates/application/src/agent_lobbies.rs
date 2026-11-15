use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, PrincipalId, RoomCatalogId, RoomInstanceId},
    moderation::ModerationActionKind,
    private_rooms::PrivateRoomMembershipStatus,
    rooms::{MatrixRoomReference, RoomCatalogKind, RoomInstance, RoomLanguage, RoomRegion},
};

use crate::{
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentLobbyAccessRecord, AgentLobbyAccessRepository, AgentRoomMembershipFactory, Clock,
        MatrixFailure, MatrixFailureKind, MatrixRoomId, ModerationEffectGateway,
        ModerationEffectTarget, PortFuture, PrivateMatrixMembership, PrivateRoomAgentAccessStore,
        PrivateRoomMatrixGateway, PrivateRoomStore, RoomAllocationEvidence, RoomAllocationMode,
        RoomAllocationStore, StandingModeration, StandingModerationSource,
    },
    private_rooms::{
        AgentAccessFailure, AgentAccessFailureKind, AgentMemberSeat, AgentOwnerSeat,
        PrivateRoomFailure, PrivateRoomFailureKind, SubRoomAgentMemberSeating,
        SubRoomAgentOwnerSeating,
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
    /// 私人房间不接这个 Agent：房间已归档，或者它代表的账号没加入、不能发言，它自己也没凭口令进来过。
    /// 是房间此刻的状态，不是连接或设备出了问题（agent-room-ops#74）。
    RoomRefused,
    /// 进的是有门的子房间，它的主人不在里面：要主人先申请，或者等管理员把他请进来
    /// （`specs/redesign/sub-rooms.md` 的“进门”，2f-1a）。
    OwnerNotInside,
    /// 进的是有门的子房间，这个 Agent 是凭口令或敲门进的父房间（主人带不了它进来）：要管得了这一间的
    /// 人按名字放它进来（2f-2b-2）。
    AgentNotAdmitted,
    NotFound,
    Conflict,
    DependencyUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLobbyEntryFailure {
    Unauthorized,
    RoomRefused,
    OwnerNotInside,
    AgentNotAdmitted,
    NotFound,
    Access(RepositoryError),
    Membership(MatrixFailure),
    Lobby(EnterLobbyFailure),
    /// 没有门的子房间里替主人占位置时出了错。
    SubRoomOwner(PrivateRoomFailure),
    /// 把凭口令进了父房间的 Agent 记成子房间的 Agent 成员时出了错。
    SubRoomAgentMember(AgentAccessFailure),
}

impl AgentLobbyEntryFailure {
    pub const fn kind(&self) -> AgentLobbyEntryFailureKind {
        match self {
            Self::Unauthorized => AgentLobbyEntryFailureKind::Unauthorized,
            Self::RoomRefused => AgentLobbyEntryFailureKind::RoomRefused,
            Self::OwnerNotInside => AgentLobbyEntryFailureKind::OwnerNotInside,
            Self::AgentNotAdmitted => AgentLobbyEntryFailureKind::AgentNotAdmitted,
            Self::NotFound => AgentLobbyEntryFailureKind::NotFound,
            Self::Access(error) => repository_failure_kind(error.kind()),
            Self::Membership(error) => matrix_failure_kind(error.kind()),
            Self::Lobby(error) => lobby_failure_kind(error),
            Self::SubRoomOwner(failure) => private_room_failure_kind(failure.kind()),
            Self::SubRoomAgentMember(failure) => agent_access_failure_kind(failure.kind()),
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

/// 主人从“我的 Agent”把 Agent 带进房间：服务器替它的一个连接进房间，不要设备签名。
///
/// 调用方已经核对过主人和这个连接；这里照旧按连接代表的账号裁决私人房间的入场资格，和 Agent
/// 自己进来一样。
pub trait AgentInstanceRoomEntry: Send + Sync {
    fn enter_instance<'a>(
        &'a self,
        access: &'a AgentLobbyAccessRecord,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'a, AgentLobbyEntryResult<EnterLobbyOutcome>>;
}

pub struct AgentLobbyEntryDependencies {
    pub access: Arc<dyn AgentLobbyAccessRepository>,
    pub allocations: Arc<dyn RoomAllocationStore>,
    pub private_rooms: Arc<dyn PrivateRoomStore>,
    /// 凭口令进来的 Agent 成员：它们不随任何成员入场。
    pub private_agents: Arc<dyn PrivateRoomAgentAccessStore>,
    /// 本机 Agent 进没有门的子房间时，先把还不在里面的主人加进去。
    pub sub_room_owners: Arc<dyn SubRoomAgentOwnerSeating>,
    /// 主人带不了它进来、它自己是父房间里的 Agent 成员时，照门把它记成子房间的 Agent 成员。
    pub sub_room_agent_members: Arc<dyn SubRoomAgentMemberSeating>,
    pub private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub memberships: Arc<dyn AgentRoomMembershipFactory>,
    pub provisioning: Arc<dyn LobbyProvisioningOperation>,
    pub identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    /// Agent 代表的人此刻在房间里生效的禁言：人被禁言时，他的 Agent 进来、重连也不能说话。
    pub standing_moderation: Arc<dyn StandingModerationSource>,
    /// 公共大厅里照那条禁言把进来的 Agent 压下去。
    pub moderation_effects: Arc<dyn ModerationEffectGateway>,
    pub clock: Arc<dyn Clock>,
}

/// 进的是哪种房间。私人房间在准入时就照主人的禁言给不给发言级别；公共大厅谁都能说话，进去以后再照
/// 那条禁言把它压下去。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination {
    PublicLobby,
    PrivateRoom,
}

/// 在设备、Agent 实例与 Matrix 身份三者严格绑定后执行现有大厅 Saga。
pub struct AgentLobbyEntryService {
    access: Arc<dyn AgentLobbyAccessRepository>,
    allocations: Arc<dyn RoomAllocationStore>,
    private_rooms: Arc<dyn PrivateRoomStore>,
    private_agents: Arc<dyn PrivateRoomAgentAccessStore>,
    sub_room_owners: Arc<dyn SubRoomAgentOwnerSeating>,
    sub_room_agent_members: Arc<dyn SubRoomAgentMemberSeating>,
    private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    memberships: Arc<dyn AgentRoomMembershipFactory>,
    provisioning: Arc<dyn LobbyProvisioningOperation>,
    identifiers: Arc<dyn RoomReservationIdentifierFactory>,
    standing_moderation: Arc<dyn StandingModerationSource>,
    moderation_effects: Arc<dyn ModerationEffectGateway>,
    clock: Arc<dyn Clock>,
    join_policy: LobbyJoinPolicy,
}

impl AgentLobbyEntryService {
    pub fn new(dependencies: AgentLobbyEntryDependencies, join_policy: LobbyJoinPolicy) -> Self {
        Self {
            access: dependencies.access,
            allocations: dependencies.allocations,
            private_rooms: dependencies.private_rooms,
            private_agents: dependencies.private_agents,
            sub_room_owners: dependencies.sub_room_owners,
            sub_room_agent_members: dependencies.sub_room_agent_members,
            private_matrix: dependencies.private_matrix,
            memberships: dependencies.memberships,
            provisioning: dependencies.provisioning,
            identifiers: dependencies.identifiers,
            standing_moderation: dependencies.standing_moderation,
            moderation_effects: dependencies.moderation_effects,
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
        let (mode, destination) = if let Some(room) = &request.target_room {
            let (target, destination) = self.resolve_target(&request, &access, room).await?;
            (RoomAllocationMode::Manual(target), destination)
        } else {
            (RoomAllocationMode::Automatic, Destination::PublicLobby)
        };
        self.join(
            &access,
            destination,
            JoinLobbyRequest {
                agent_id: request.agent_id,
                agent_instance_id: request.agent_instance_id,
                catalog_id: request.catalog_id,
                mode,
                preferred_language: request.preferred_language,
                preferred_region: request.preferred_region,
                evidence: RoomAllocationEvidence::default(),
            },
        )
        .await
    }

    /// 主人带进房间：私人房间按目录找到唯一的实例，照 Agent 自己指名进来的那套裁决；别的交给
    /// 自动分配，分配只认公共大厅（私人房间绝不自动分配，见 `RoomAllocationStore`）。
    async fn enter_instance_internal(
        &self,
        access: &AgentLobbyAccessRecord,
        catalog_id: RoomCatalogId,
    ) -> AgentLobbyEntryResult<EnterLobbyOutcome> {
        if !access.active {
            return Err(AgentLobbyEntryFailure::Unauthorized);
        }
        let (mode, destination) = match self
            .private_rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
        {
            Some(snapshot) => (
                RoomAllocationMode::Manual(self.admit_private(&snapshot, access).await?),
                Destination::PrivateRoom,
            ),
            None => (RoomAllocationMode::Automatic, Destination::PublicLobby),
        };
        self.join(
            access,
            destination,
            JoinLobbyRequest {
                agent_id: access.agent_id,
                agent_instance_id: access.agent_instance_id,
                catalog_id,
                mode,
                preferred_language: None,
                preferred_region: None,
                evidence: RoomAllocationEvidence::default(),
            },
        )
        .await
    }

    /// 预约位置、在 Matrix 上加入、确认：Agent 自己进来和主人带进来共用。进了公共大厅以后，它代表的人
    /// 在这里被禁言着就把它也压下去。
    async fn join(
        &self,
        access: &AgentLobbyAccessRecord,
        destination: Destination,
        request: JoinLobbyRequest,
    ) -> AgentLobbyEntryResult<EnterLobbyOutcome> {
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
        let outcome = EnterLobbyService::new(EnterLobbyDependencies {
            joins,
            provisioning: self.provisioning.clone(),
        })
        .enter(request)
        .await
        .map_err(AgentLobbyEntryFailure::Lobby)?;
        if destination == Destination::PublicLobby
            && let EnterLobbyOutcome::Joined { room, .. } = &outcome
        {
            self.follow_mute_in_lobby(access, room).await?;
        }
        Ok(outcome)
    }

    /// 它代表的人在这个大厅里被禁言着：在它进的这个分片上照那条禁言把它也压下去。已经压着的（比如
    /// 重连）什么也不写。
    async fn follow_mute_in_lobby(
        &self,
        access: &AgentLobbyAccessRecord,
        room: &RoomInstance,
    ) -> AgentLobbyEntryResult<()> {
        let Some(mute) = self
            .standing_mute(room.catalog_id(), access.principal_id)
            .await?
        else {
            return Ok(());
        };
        let target = ModerationEffectTarget {
            matrix_room_id: MatrixRoomId::new(room.matrix_room_id().as_str().to_owned())
                .map_err(|_| AgentLobbyEntryFailure::NotFound)?,
            room_kind: RoomCatalogKind::PublicLobby,
            target: mute.action.target().clone(),
            target_matrix_user_id: Some(mute.target_matrix_user_id),
            target_agent_matrix_user_ids: vec![access.matrix_user_id.clone()],
        };
        self.moderation_effects
            .apply(&mute.action, &target)
            .await
            .map_err(AgentLobbyEntryFailure::Membership)
    }

    /// 这个人在这个房间里此刻生效的禁言，最早的那条。治理按 UUID 认人，写法不一样也是同一个人。
    async fn standing_mute(
        &self,
        catalog_id: RoomCatalogId,
        principal_id: PrincipalId,
    ) -> AgentLobbyEntryResult<Option<StandingModeration>> {
        let standing = self
            .standing_moderation
            .standing_person_actions(catalog_id, self.clock.now())
            .await
            .map_err(AgentLobbyEntryFailure::Access)?;
        Ok(standing.into_iter().find(|item| {
            item.action.kind() == ModerationActionKind::Mute
                && uuid::Uuid::parse_str(item.action.target().reference())
                    .is_ok_and(|id| id == principal_id.as_uuid())
        }))
    }

    /// 解析指名房间。公共大厅按目录可见性放行，其余交给私人房间的成员事实裁决。
    async fn resolve_target(
        &self,
        request: &EnterAgentLobby,
        access: &AgentLobbyAccessRecord,
        room: &MatrixRoomReference,
    ) -> AgentLobbyEntryResult<(RoomInstanceId, Destination)> {
        if let Some(target) = self
            .access
            .find_public_lobby_room(request.catalog_id, room)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
        {
            return Ok((target, Destination::PublicLobby));
        }
        let target = self.resolve_private_target(request, access, room).await?;
        Ok((target, Destination::PrivateRoom))
    }

    /// 私人房间不在公共目录里，Agent 随它此次代表的主体入场，或者它本身是凭口令进来的 Agent 成员。
    ///
    /// 随主体入场时能力不超过该主体：只有已加入且可发言的成员才能带 Agent 进来，移除或封禁该成员
    /// 会立即让其 Agent 失去房间。凭口令进来的 Agent 不随任何成员进出，由房主或管理员单独移出。
    /// 目录必须与请求一致，避免用另一个房间的成员资格换取本房间的入场。
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
        self.admit_private(&snapshot, access).await
    }

    /// 私人房间的准入：Agent 代表的账号已加入且能发言，或者它本身是还有效的 Agent 成员。准了就在
    /// Matrix 上邀请它、给它发言级别，返回房间唯一的实例。
    async fn admit_private(
        &self,
        snapshot: &crate::ports::PrivateRoomSnapshot,
        access: &AgentLobbyAccessRecord,
    ) -> AgentLobbyEntryResult<RoomInstanceId> {
        if !snapshot.room().admits_agent_of(access.principal_id)
            && !self.admitted_by_code(snapshot, access.agent_id).await?
        {
            self.seat_in_sub_room(snapshot, access).await?;
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
        // 私人房间默认只让成员旁观；Agent 要上线（发布状态）和回复都需要发言级别。上面的准入
        // 已确认它代表的主体此刻已加入且能发言，或它本身是能发言的 Agent 成员，所以按同样的能力
        // 授予。主体失去发言、被移除或离开时由私人房间服务收回，Agent 成员被移出时由口令服务收回。
        // 它代表的人在这里被禁言着就不给：不然 Agent 一重连就把禁言冲掉了，禁言撤了由治理一起放开。
        // 正在落的禁言也算，子房间里连父房间的一起算。重连时重复授予不会再写权限状态。
        let muted = self
            .standing_moderation
            .person_muted(
                snapshot.catalog().id(),
                access.principal_id,
                self.clock.now(),
            )
            .await
            .map_err(AgentLobbyEntryFailure::Access)?;
        self.private_matrix
            .set_speaking(&matrix_room, &access.matrix_user_id, !muted)
            .await
            .map_err(AgentLobbyEntryFailure::Membership)?;
        Ok(snapshot.instance().id())
    }

    /// 主人此刻不在房间里、Agent 也不是这一间的 Agent 成员时：是没有门的子房间，就先把主人加进去
    /// （2f-1a）；主人带不了它进来的，它自己是父房间里的 Agent 成员就照门把它记成这一间的 Agent 成员
    /// （2f-2b-2）。别的照旧不接。有门的说清楚要主人先申请，或者要管理者按名字放它进来。
    async fn seat_in_sub_room(
        &self,
        snapshot: &crate::ports::PrivateRoomSnapshot,
        access: &AgentLobbyAccessRecord,
    ) -> AgentLobbyEntryResult<()> {
        if snapshot.parent_catalog_id().is_none() {
            return Err(AgentLobbyEntryFailure::RoomRefused);
        }
        match self
            .sub_room_owners
            .seat_agent_owner(snapshot.catalog().id(), access.principal_id)
            .await
            .map_err(AgentLobbyEntryFailure::SubRoomOwner)?
        {
            AgentOwnerSeat::Inside => Ok(()),
            AgentOwnerSeat::DoorClosed => Err(AgentLobbyEntryFailure::OwnerNotInside),
            AgentOwnerSeat::Refused => self.seat_agent_member(snapshot, access).await,
        }
    }

    /// 凭口令或敲门进了父房间的本机 Agent 自己进子房间（2f-2b-2）。主人在这一间里被封禁着的不接：
    /// 封禁连人带他的 Agent 一起算，它没进来过就没有被一起移出的记录。
    async fn seat_agent_member(
        &self,
        snapshot: &crate::ports::PrivateRoomSnapshot,
        access: &AgentLobbyAccessRecord,
    ) -> AgentLobbyEntryResult<()> {
        if snapshot
            .room()
            .member(access.principal_id)
            .is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned)
        {
            return Err(AgentLobbyEntryFailure::RoomRefused);
        }
        match self
            .sub_room_agent_members
            .seat_agent_member(snapshot.catalog().id(), access.agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::SubRoomAgentMember)?
        {
            AgentMemberSeat::Inside => Ok(()),
            AgentMemberSeat::DoorClosed => Err(AgentLobbyEntryFailure::AgentNotAdmitted),
            AgentMemberSeat::Refused => Err(AgentLobbyEntryFailure::RoomRefused),
        }
    }

    /// 这个 Agent 本身是凭口令进来、仍然有效的 Agent 成员。它的主人不必是房间成员。
    async fn admitted_by_code(
        &self,
        snapshot: &crate::ports::PrivateRoomSnapshot,
        agent_id: AgentId,
    ) -> AgentLobbyEntryResult<bool> {
        let member = self
            .private_agents
            .agent_member(snapshot.catalog().id(), agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?;
        Ok(snapshot
            .room()
            .admits_agent_member(member.is_some_and(|member| {
                member.status == agent_room_domain::join_codes::PrivateRoomAgentMemberStatus::Joined
            })))
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

impl AgentInstanceRoomEntry for AgentLobbyEntryService {
    fn enter_instance<'a>(
        &'a self,
        access: &'a AgentLobbyAccessRecord,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'a, AgentLobbyEntryResult<EnterLobbyOutcome>> {
        Box::pin(self.enter_instance_internal(access, catalog_id))
    }
}

const fn private_room_failure_kind(kind: PrivateRoomFailureKind) -> AgentLobbyEntryFailureKind {
    match kind {
        PrivateRoomFailureKind::NotFound => AgentLobbyEntryFailureKind::NotFound,
        PrivateRoomFailureKind::Conflict => AgentLobbyEntryFailureKind::Conflict,
        PrivateRoomFailureKind::DependencyUnavailable => {
            AgentLobbyEntryFailureKind::DependencyUnavailable
        }
        PrivateRoomFailureKind::UnknownCommit => AgentLobbyEntryFailureKind::UnknownCommit,
        PrivateRoomFailureKind::Forbidden | PrivateRoomFailureKind::SubRoom(_) => {
            AgentLobbyEntryFailureKind::RoomRefused
        }
        PrivateRoomFailureKind::InvalidRequest
        | PrivateRoomFailureKind::OwnerOnly
        | PrivateRoomFailureKind::ReauthenticationRequired
        | PrivateRoomFailureKind::PublicRoomLimitReached
        | PrivateRoomFailureKind::Internal => AgentLobbyEntryFailureKind::Internal,
    }
}

const fn agent_access_failure_kind(kind: AgentAccessFailureKind) -> AgentLobbyEntryFailureKind {
    match kind {
        AgentAccessFailureKind::NotFound => AgentLobbyEntryFailureKind::NotFound,
        AgentAccessFailureKind::Conflict => AgentLobbyEntryFailureKind::Conflict,
        AgentAccessFailureKind::DependencyUnavailable | AgentAccessFailureKind::RateLimited => {
            AgentLobbyEntryFailureKind::DependencyUnavailable
        }
        AgentAccessFailureKind::UnknownCommit => AgentLobbyEntryFailureKind::UnknownCommit,
        AgentAccessFailureKind::Forbidden
        | AgentAccessFailureKind::ClosedToNetworkAgents
        | AgentAccessFailureKind::ParentOnly => AgentLobbyEntryFailureKind::RoomRefused,
        AgentAccessFailureKind::InviteOnly => AgentLobbyEntryFailureKind::AgentNotAdmitted,
        AgentAccessFailureKind::InvalidRequest
        | AgentAccessFailureKind::NotRevealable
        | AgentAccessFailureKind::Internal => AgentLobbyEntryFailureKind::Internal,
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
