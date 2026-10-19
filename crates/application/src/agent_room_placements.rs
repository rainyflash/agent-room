//! Agent 在哪些房间、主人把它带进房间、离开一个房间（`specs/agent-identity/design.md` 第 2 步）。
//!
//! 带进房间由主人用网页登录发起，服务器替 Agent 在主人这台电脑上的连接进房间，和 Agent 自己进来
//! 走同一套（私人房间看连接代表的账号能不能带 Agent 进来）。
//!
//! 离开由 Agent 自己（Bridge 用设备签名）或主人（网页登录）发起，做三件事：私人房间里收回它的
//! 发言级别、以应用服务的身份让它在 Matrix 上离开、放掉服务器上给它占的位置。凭口令或敲门进来的，
//! 再删掉它的 Agent 成员记录，以后要重新凭口令或敲门。

use std::sync::Arc;

use agent_room_domain::{
    agents::AgentRole,
    ids::{AgentId, AgentInstanceId, DeviceId, RoomCatalogId},
    rooms::{MatrixRoomReference, RoomCatalogStatus, RoomReservationState},
    time::UtcMillis,
};

use crate::{
    agent_lobbies::{AgentInstanceRoomEntry, AgentLobbyEntryFailure, AgentLobbyEntryResult},
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    ports::{
        AgentLobbyAccessRepository, AgentMembershipRepository, AgentRoomKind,
        AgentRoomMembershipFactory, AgentRoomPlacement, AgentRoomPlacementStore, Clock,
        MatrixFailure, MatrixFailureKind, MatrixRoomId, MatrixUserId, PortFuture,
        PrivateRoomMatrixGateway, RoomAllocationStore,
    },
    rooms::EnterLobbyOutcome,
};

/// 谁在让 Agent 离开。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRoomActor {
    /// Agent 自己：这台电脑上驱动它的那个连接。
    Device {
        device: AuthenticatedDevice,
        agent_instance_id: AgentInstanceId,
    },
    /// 主人：Agent 的主人或操作者，用网页登录。
    Owner(AuthenticatedPrincipal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAgentRooms {
    pub actor: AuthenticatedPrincipal,
    pub agent_id: AgentId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveAgentRoom {
    pub actor: AgentRoomActor,
    pub agent_id: AgentId,
    pub catalog_id: RoomCatalogId,
}

/// Agent 在的一个房间。同一个 Agent 有几个连接在同一个房间时只算一个。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRoom {
    pub catalog_id: RoomCatalogId,
    pub kind: AgentRoomKind,
    pub name: String,
    pub status: RoomCatalogStatus,
}

/// 主人从“我的 Agent”把它带进一个房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BringAgentIntoRoom {
    pub actor: AuthenticatedPrincipal,
    pub agent_id: AgentId,
    pub catalog_id: RoomCatalogId,
}

/// 带进房间的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroughtAgent {
    /// 进去了几个连接。是 0 时它在你的电脑上还没有能用的连接：没连过，或者连接都失效了。
    Entered(usize),
    /// 公共大厅正在开新的分片，或者刚选好的分片满了，到 `retry_at` 再来。
    Busy { retry_at: UtcMillis },
}

/// 删除 Agent 之前让它离开它在的所有房间。只有主人能做，和删除 Agent 一样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveAllAgentRooms {
    pub actor: AuthenticatedPrincipal,
    pub agent_id: AgentId,
}

/// 离开的结果。原本就不在这个房间时 `left` 为 `false`，也算成功：重复点“离开”不报错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeftAgentRoom {
    pub left: bool,
}

pub trait AgentRoomPlacementUseCases: Send + Sync {
    fn rooms(
        &self,
        request: ListAgentRooms,
    ) -> PortFuture<'_, AgentLobbyEntryResult<Vec<AgentRoom>>>;

    fn bring(
        &self,
        request: BringAgentIntoRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<BroughtAgent>>;

    fn leave(
        &self,
        request: LeaveAgentRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<LeftAgentRoom>>;

    /// 离开所有房间，返回离开了几个。
    fn leave_all(
        &self,
        request: LeaveAllAgentRooms,
    ) -> PortFuture<'_, AgentLobbyEntryResult<usize>>;
}

/// 主人这边要做的事要哪种角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerAccess {
    /// 看在哪些房间：Agent 的成员都行。
    View,
    /// 带进房间：主人或操作者，只带他自己电脑上的连接。
    Bring,
    /// 让它离开一个房间：主人或操作者。
    Leave,
    /// 删除之前离开所有房间：只有主人。
    Retire,
}

pub struct AgentRoomPlacementDependencies {
    pub access: Arc<dyn AgentLobbyAccessRepository>,
    pub agent_memberships: Arc<dyn AgentMembershipRepository>,
    pub placements: Arc<dyn AgentRoomPlacementStore>,
    pub allocations: Arc<dyn RoomAllocationStore>,
    pub private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub memberships: Arc<dyn AgentRoomMembershipFactory>,
    /// 带进房间时替连接进房间，和 Agent 自己进来是同一个服务。
    pub entry: Arc<dyn AgentInstanceRoomEntry>,
    pub clock: Arc<dyn Clock>,
}

pub struct AgentRoomPlacementService {
    access: Arc<dyn AgentLobbyAccessRepository>,
    agent_memberships: Arc<dyn AgentMembershipRepository>,
    placements: Arc<dyn AgentRoomPlacementStore>,
    allocations: Arc<dyn RoomAllocationStore>,
    private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    memberships: Arc<dyn AgentRoomMembershipFactory>,
    entry: Arc<dyn AgentInstanceRoomEntry>,
    clock: Arc<dyn Clock>,
}

impl AgentRoomPlacementService {
    pub fn new(dependencies: AgentRoomPlacementDependencies) -> Self {
        Self {
            access: dependencies.access,
            agent_memberships: dependencies.agent_memberships,
            placements: dependencies.placements,
            allocations: dependencies.allocations,
            private_matrix: dependencies.private_matrix,
            memberships: dependencies.memberships,
            entry: dependencies.entry,
            clock: dependencies.clock,
        }
    }

    async fn rooms_internal(
        &self,
        request: ListAgentRooms,
    ) -> AgentLobbyEntryResult<Vec<AgentRoom>> {
        self.authorize_owner(&request.actor, request.agent_id, OwnerAccess::View)
            .await?;
        let placements = self
            .placements
            .placements(request.agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?;
        let mut rooms: Vec<AgentRoom> = Vec::with_capacity(placements.len());
        for placement in placements {
            if rooms
                .iter()
                .any(|room| room.catalog_id == placement.catalog_id)
            {
                continue;
            }
            rooms.push(AgentRoom {
                catalog_id: placement.catalog_id,
                kind: placement.kind,
                name: placement.name,
                status: placement.status,
            });
        }
        Ok(rooms)
    }

    /// 只带主人自己电脑上的连接：连接代表的是它所在设备的账号，私人房间按这个账号裁决，带别人
    /// 电脑上的连接等于替别人进房间。同一台设备只带最新的那个连接，旧的多半已经不用了。
    async fn bring_internal(
        &self,
        request: BringAgentIntoRoom,
    ) -> AgentLobbyEntryResult<BroughtAgent> {
        self.authorize_owner(&request.actor, request.agent_id, OwnerAccess::Bring)
            .await?;
        let accesses = self
            .access
            .lobby_accesses_of_agent(request.agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?;
        let mut devices: Vec<DeviceId> = Vec::new();
        for access in accesses {
            if !access.active
                || access.agent_id != request.agent_id
                || access.principal_id != request.actor.principal_id
                || devices.contains(&access.device_id)
            {
                continue;
            }
            devices.push(access.device_id);
            match self
                .entry
                .enter_instance(&access, request.catalog_id)
                .await?
            {
                EnterLobbyOutcome::Joined { .. } => {}
                EnterLobbyOutcome::ProvisioningBusy { retry_at } => {
                    return Ok(BroughtAgent::Busy { retry_at });
                }
                EnterLobbyOutcome::CapacityChanged { .. } => {
                    return Ok(BroughtAgent::Busy {
                        retry_at: self.clock.now(),
                    });
                }
            }
        }
        Ok(BroughtAgent::Entered(devices.len()))
    }

    async fn leave_internal(
        &self,
        request: LeaveAgentRoom,
    ) -> AgentLobbyEntryResult<LeftAgentRoom> {
        match &request.actor {
            AgentRoomActor::Device {
                device,
                agent_instance_id,
            } => {
                self.authorize_device(device, *agent_instance_id, request.agent_id)
                    .await?;
            }
            AgentRoomActor::Owner(actor) => {
                self.authorize_owner(actor, request.agent_id, OwnerAccess::Leave)
                    .await?;
            }
        }
        let placements: Vec<AgentRoomPlacement> = self
            .placements
            .placements(request.agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .into_iter()
            .filter(|placement| placement.catalog_id == request.catalog_id)
            .collect();
        if placements.is_empty() {
            return Ok(LeftAgentRoom { left: false });
        }
        let matrix_user = self.matrix_user(request.agent_id).await?;
        self.leave_room(request.agent_id, &placements, &matrix_user)
            .await?;
        Ok(LeftAgentRoom { left: true })
    }

    async fn leave_all_internal(
        &self,
        request: LeaveAllAgentRooms,
    ) -> AgentLobbyEntryResult<usize> {
        self.authorize_owner(&request.actor, request.agent_id, OwnerAccess::Retire)
            .await?;
        let placements = self
            .placements
            .placements(request.agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?;
        if placements.is_empty() {
            return Ok(0);
        }
        let matrix_user = self.matrix_user(request.agent_id).await?;
        let mut catalogs: Vec<RoomCatalogId> = Vec::new();
        for placement in &placements {
            if !catalogs.contains(&placement.catalog_id) {
                catalogs.push(placement.catalog_id);
            }
        }
        for catalog_id in &catalogs {
            let room: Vec<AgentRoomPlacement> = placements
                .iter()
                .filter(|placement| placement.catalog_id == *catalog_id)
                .cloned()
                .collect();
            self.leave_room(request.agent_id, &room, &matrix_user)
                .await?;
        }
        Ok(catalogs.len())
    }

    async fn matrix_user(&self, agent_id: AgentId) -> AgentLobbyEntryResult<MatrixUserId> {
        self.placements
            .matrix_user(agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .ok_or(AgentLobbyEntryFailure::NotFound)
    }

    /// 离开一个房间：`placements` 是它在这个房间占着的全部位置。
    async fn leave_room(
        &self,
        agent_id: AgentId,
        placements: &[AgentRoomPlacement],
        matrix_user: &MatrixUserId,
    ) -> AgentLobbyEntryResult<()> {
        let Some(room) = placements.first() else {
            return Ok(());
        };
        self.leave_matrix(room, matrix_user).await?;
        if room.kind == AgentRoomKind::PrivateRoom {
            self.placements
                .forget_agent_member(room.catalog_id, agent_id)
                .await
                .map_err(AgentLobbyEntryFailure::Access)?;
        }
        let now = self.clock.now();
        for placement in placements {
            self.allocations
                .transition(
                    placement.reservation_id,
                    RoomReservationState::Committed,
                    RoomReservationState::Released,
                    now,
                )
                .await
                .map_err(AgentLobbyEntryFailure::Access)?;
        }
        Ok(())
    }

    async fn leave_matrix(
        &self,
        room: &AgentRoomPlacement,
        matrix_user: &MatrixUserId,
    ) -> AgentLobbyEntryResult<()> {
        leave_matrix_room(
            self.private_matrix.as_ref(),
            self.memberships.as_ref(),
            MatrixRoomToLeave {
                kind: room.kind,
                status: room.status,
                matrix_room_id: &room.matrix_room_id,
            },
            matrix_user,
        )
        .await
        .map_err(|failure| match failure {
            RoomLeaveFailure::InvalidRoom => AgentLobbyEntryFailure::NotFound,
            RoomLeaveFailure::Matrix(failure) => AgentLobbyEntryFailure::Membership(failure),
        })
    }

    /// Agent 自己离开：设备、连接和 Agent 三者对得上，而且连接还有效，和它进房间时一样。
    async fn authorize_device(
        &self,
        device: &AuthenticatedDevice,
        agent_instance_id: AgentInstanceId,
        agent_id: AgentId,
    ) -> AgentLobbyEntryResult<()> {
        if device.access_token_expires_at <= self.clock.now() {
            return Err(AgentLobbyEntryFailure::Unauthorized);
        }
        let access = self
            .access
            .find_lobby_access(agent_instance_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .ok_or(AgentLobbyEntryFailure::NotFound)?;
        if access.active && access.agent_id == agent_id && access.device_id == device.device_id {
            Ok(())
        } else {
            Err(AgentLobbyEntryFailure::Unauthorized)
        }
    }

    /// 主人这边：看在哪些房间，Agent 的成员都行；带进房间、让它离开要主人或操作者；删除之前离开
    /// 所有房间只有主人能做。不是成员的当作没有这个 Agent，不让人拿 ID 试探别人的 Agent。
    async fn authorize_owner(
        &self,
        actor: &AuthenticatedPrincipal,
        agent_id: AgentId,
        access: OwnerAccess,
    ) -> AgentLobbyEntryResult<()> {
        if actor.expires_at <= self.clock.now() {
            return Err(AgentLobbyEntryFailure::Unauthorized);
        }
        let role = self
            .agent_memberships
            .find_memberships(agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .and_then(|memberships| memberships.role_of(actor.principal_id))
            .ok_or(AgentLobbyEntryFailure::NotFound)?;
        let allowed = match access {
            OwnerAccess::View => true,
            OwnerAccess::Bring | OwnerAccess::Leave => role.can_register_instance(),
            OwnerAccess::Retire => role == AgentRole::Owner,
        };
        if allowed {
            Ok(())
        } else {
            Err(AgentLobbyEntryFailure::Unauthorized)
        }
    }
}

/// 要离开的一个 Matrix 房间。
#[derive(Debug, Clone, Copy)]
pub(crate) struct MatrixRoomToLeave<'a> {
    pub(crate) kind: AgentRoomKind,
    pub(crate) status: RoomCatalogStatus,
    pub(crate) matrix_room_id: &'a MatrixRoomReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomLeaveFailure {
    /// 记着的 Matrix 房间号不对。
    InvalidRoom,
    Matrix(MatrixFailure),
}

/// 以应用服务的身份让 Agent 离开一个 Matrix 房间。离开一个房间、删除 Agent 前离开所有房间、撤销电脑后
/// 离开（`agent_room_departures`）都用它。
///
/// 私人房间先收回发言级别再离开：离开以后 Matrix 上的发言级别还留着，下次再进来时由进房间那一套
/// 按当时的准入重新给。归档了的房间不动发言级别：写发言级别会把整套在用的门槛写回去，等于取消归档。
///
/// 在 Matrix 上已经不在房间里（被移出、主人离开时被一起请出去、或者上次离开到一半）时，离开会被
/// 拒绝（Synapse 回 403，房间不认得时 404），这里当作已经离开，接着放掉位置。
pub(crate) async fn leave_matrix_room(
    private_matrix: &dyn PrivateRoomMatrixGateway,
    memberships: &dyn AgentRoomMembershipFactory,
    room: MatrixRoomToLeave<'_>,
    matrix_user: &MatrixUserId,
) -> Result<(), RoomLeaveFailure> {
    if room.kind == AgentRoomKind::PrivateRoom && room.status == RoomCatalogStatus::Active {
        let matrix_room = MatrixRoomId::new(room.matrix_room_id.as_str().to_owned())
            .map_err(|_| RoomLeaveFailure::InvalidRoom)?;
        private_matrix
            .set_speaking(&matrix_room, matrix_user, false)
            .await
            .map_err(RoomLeaveFailure::Matrix)?;
    }
    let membership = memberships
        .bind(matrix_user)
        .map_err(RoomLeaveFailure::Matrix)?;
    match membership.leave(room.matrix_room_id).await {
        Ok(()) => Ok(()),
        Err(failure) if already_gone(&failure) => Ok(()),
        Err(failure) => Err(RoomLeaveFailure::Matrix(failure)),
    }
}

fn already_gone(failure: &MatrixFailure) -> bool {
    matches!(
        failure.kind(),
        MatrixFailureKind::Forbidden | MatrixFailureKind::NotFound
    )
}

impl AgentRoomPlacementUseCases for AgentRoomPlacementService {
    fn rooms(
        &self,
        request: ListAgentRooms,
    ) -> PortFuture<'_, AgentLobbyEntryResult<Vec<AgentRoom>>> {
        Box::pin(self.rooms_internal(request))
    }

    fn bring(
        &self,
        request: BringAgentIntoRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<BroughtAgent>> {
        Box::pin(self.bring_internal(request))
    }

    fn leave(
        &self,
        request: LeaveAgentRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<LeftAgentRoom>> {
        Box::pin(self.leave_internal(request))
    }

    fn leave_all(
        &self,
        request: LeaveAllAgentRooms,
    ) -> PortFuture<'_, AgentLobbyEntryResult<usize>> {
        Box::pin(self.leave_all_internal(request))
    }
}
