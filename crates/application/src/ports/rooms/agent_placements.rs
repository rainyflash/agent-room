use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomCatalogStatus},
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixUserId, PortFuture},
};

/// Agent 占着位置的房间是公共大厅还是私人房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRoomKind {
    PublicLobby,
    PrivateRoom,
}

/// Agent 的一个连接在一个房间里占着的位置（已确认的那种），连同房间的名字和状态
/// （specs/agent-identity/design.md）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRoomPlacement {
    pub reservation_id: RoomReservationId,
    pub agent_instance_id: AgentInstanceId,
    pub catalog_id: RoomCatalogId,
    pub room_instance_id: RoomInstanceId,
    pub matrix_room_id: MatrixRoomReference,
    pub kind: AgentRoomKind,
    pub name: String,
    pub status: RoomCatalogStatus,
}

/// Agent 在哪些房间、离开一个房间时要用的几样事实。
pub trait AgentRoomPlacementStore: Send + Sync {
    /// Agent 每个连接已确认占着的位置（只算公共大厅和私人房间），按进房间的时间排。
    fn placements(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentRoomPlacement>>>;

    /// Agent 的 Matrix 账号；没有这个 Agent 时为空。
    fn matrix_user(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>>;

    /// 删掉它在这个私人房间凭口令或敲门进来、还算已加入的成员记录；没有时返回 `false`。
    ///
    /// 不记成已移出：那是管理者移出用的，记了以后它拿原来的口令就进不来了。
    fn forget_agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<bool>>;
}
