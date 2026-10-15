//! Agent 在哪些房间、离开一个房间（`specs/agent-identity/design.md` 第 2 步）。
//!
//! 离开由 Agent 自己（Bridge 用设备签名）或主人（网页登录）发起，做三件事：私人房间里收回它的
//! 发言级别、以应用服务的身份让它在 Matrix 上离开、放掉服务器上给它占的位置。凭口令或敲门进来的，
//! 再删掉它的 Agent 成员记录，以后要重新凭口令或敲门。

use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    rooms::{RoomCatalogStatus, RoomReservationState},
};

use crate::{
    agent_lobbies::{AgentLobbyEntryFailure, AgentLobbyEntryResult},
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    ports::{
        AgentLobbyAccessRepository, AgentMembershipRepository, AgentRoomKind,
        AgentRoomMembershipFactory, AgentRoomPlacement, AgentRoomPlacementStore, Clock,
        MatrixFailure, MatrixFailureKind, MatrixRoomId, MatrixUserId, PortFuture,
        PrivateRoomMatrixGateway, RoomAllocationStore,
    },
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

    fn leave(
        &self,
        request: LeaveAgentRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<LeftAgentRoom>>;
}

pub struct AgentRoomPlacementDependencies {
    pub access: Arc<dyn AgentLobbyAccessRepository>,
    pub agent_memberships: Arc<dyn AgentMembershipRepository>,
    pub placements: Arc<dyn AgentRoomPlacementStore>,
    pub allocations: Arc<dyn RoomAllocationStore>,
    pub private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub memberships: Arc<dyn AgentRoomMembershipFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct AgentRoomPlacementService {
    access: Arc<dyn AgentLobbyAccessRepository>,
    agent_memberships: Arc<dyn AgentMembershipRepository>,
    placements: Arc<dyn AgentRoomPlacementStore>,
    allocations: Arc<dyn RoomAllocationStore>,
    private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    memberships: Arc<dyn AgentRoomMembershipFactory>,
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
            clock: dependencies.clock,
        }
    }

    async fn rooms_internal(
        &self,
        request: ListAgentRooms,
    ) -> AgentLobbyEntryResult<Vec<AgentRoom>> {
        self.authorize_owner(&request.actor, request.agent_id, false)
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
                self.authorize_owner(actor, request.agent_id, true).await?;
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
        let Some(room) = placements.first() else {
            return Ok(LeftAgentRoom { left: false });
        };
        let matrix_user = self
            .placements
            .matrix_user(request.agent_id)
            .await
            .map_err(AgentLobbyEntryFailure::Access)?
            .ok_or(AgentLobbyEntryFailure::NotFound)?;
        self.leave_matrix(room, &matrix_user).await?;
        if room.kind == AgentRoomKind::PrivateRoom {
            self.placements
                .forget_agent_member(request.catalog_id, request.agent_id)
                .await
                .map_err(AgentLobbyEntryFailure::Access)?;
        }
        let now = self.clock.now();
        for placement in &placements {
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
        Ok(LeftAgentRoom { left: true })
    }

    /// 私人房间先收回发言级别再离开：离开以后 Matrix 上的发言级别还留着，下次再进来时由进房间那一套
    /// 按当时的准入重新给。归档了的房间不动发言级别：写发言级别会把整套在用的门槛写回去，等于取消归档。
    ///
    /// 在 Matrix 上已经不在房间里（被移出、主人离开时被一起请出去、或者上次离开到一半）时，离开会被
    /// 拒绝（Synapse 回 403，房间不认得时 404），这里当作已经离开，接着放掉位置。
    async fn leave_matrix(
        &self,
        room: &AgentRoomPlacement,
        matrix_user: &MatrixUserId,
    ) -> AgentLobbyEntryResult<()> {
        if room.kind == AgentRoomKind::PrivateRoom && room.status == RoomCatalogStatus::Active {
            let matrix_room = MatrixRoomId::new(room.matrix_room_id.as_str().to_owned())
                .map_err(|_| AgentLobbyEntryFailure::NotFound)?;
            self.private_matrix
                .set_speaking(&matrix_room, matrix_user, false)
                .await
                .map_err(AgentLobbyEntryFailure::Membership)?;
        }
        let membership = self
            .memberships
            .bind(matrix_user)
            .map_err(AgentLobbyEntryFailure::Membership)?;
        match membership.leave(&room.matrix_room_id).await {
            Ok(()) => Ok(()),
            Err(failure) if already_gone(&failure) => Ok(()),
            Err(failure) => Err(AgentLobbyEntryFailure::Membership(failure)),
        }
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

    /// 主人这边：看在哪些房间，Agent 的成员都行；让它离开要主人或操作者。不是成员的当作没有这个
    /// Agent，不让人拿 ID 试探别人的 Agent。
    async fn authorize_owner(
        &self,
        actor: &AuthenticatedPrincipal,
        agent_id: AgentId,
        manage: bool,
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
        if !manage || role.can_register_instance() {
            Ok(())
        } else {
            Err(AgentLobbyEntryFailure::Unauthorized)
        }
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

    fn leave(
        &self,
        request: LeaveAgentRoom,
    ) -> PortFuture<'_, AgentLobbyEntryResult<LeftAgentRoom>> {
        Box::pin(self.leave_internal(request))
    }
}
