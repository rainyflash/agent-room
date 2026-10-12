//! 网络 Agent 自己建房间、管房间（`specs/agent-rooms/design.md` 第 4 步）里要替 Agent 进房间的两样：
//! 建房和放行敲门。建好以后和凭口令进私人房间一样：先让加密客户端就绪、建好加密身份，再进去，进了
//! 说一声在线；放行时替敲门的 Agent 进去，和管理者在网页上放行一样。只动账的几样（关房、口令、看
//! 敲门、不让进）由接口直接交给 `NetworkAgentRoomUseCases`。

use agent_room_application::{
    network_agents::{
        NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentOwnedRoom, NetworkAgentRoom,
    },
    ports::{PortFuture, PrivateRoomAgentMemberRecord},
};
use agent_room_domain::ids::AgentId;

use super::{NetworkGateway, NetworkGatewayFailure};

/// 网络 Agent 管自己建的房间里要替 Agent 进房间的；HTTP 接口与远程 MCP 只认这个，便于单独测试。
pub(crate) trait NetworkAgentRooms: Send + Sync {
    /// 建房并进去；它没关的房间里已经有同名的，就回到那间（还没进去的这次进去）。
    fn create_room<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentOwnedRoom, NetworkGatewayFailure>>;

    /// 放行在敲它房间的 Agent：先记为成员，再替那个 Agent 进房间，进去了才把敲门记为放进来了。
    /// 回答放进来的 Agent。
    fn admit_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, Result<PrivateRoomAgentMemberRecord, NetworkGatewayFailure>>;
}

impl NetworkGateway {
    async fn create_room_internal(
        &self,
        token: &str,
        name: &str,
    ) -> Result<NetworkAgentOwnedRoom, NetworkGatewayFailure> {
        let mut owned = self
            .owned_rooms
            .create_room(token, name)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        // 上次建好了却没进去（比如那时 Matrix 暂时不通），这次接着进；进过的不再切一次加密客户端，
        // 免得白白打断它正在等的消息。
        if !owned.joined {
            owned.room = self.enter_private(token, owned.room).await?;
            owned.joined = true;
            self.announce_online(token).await;
        }
        Ok(owned)
    }

    async fn admit_knock_internal(
        &self,
        token: &str,
        room: &str,
        agent_id: AgentId,
    ) -> Result<PrivateRoomAgentMemberRecord, NetworkGatewayFailure> {
        let admitted = self
            .owned_rooms
            .admit_knock(token, room, agent_id)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let entered = NetworkAgentRoom {
            catalog_id: admitted.room.catalog_id,
            matrix_room_id: admitted.room.matrix_room_id,
            name: admitted.room.name,
        };
        self.enter_admitted_internal(agent_id, entered)
            .await
            .map_err(admission_failure)?;
        if let Err(failure) = self.owned_rooms.complete_knock(token, room, agent_id).await {
            // 它已经进去了：敲门还显示在等，再放行一次会原样走完，不影响它在房间里。
            tracing::warn!(failure = ?failure.kind(), "放行的 Agent 进了房间，敲门没记成放进来了");
        }
        Ok(admitted.agent)
    }
}

/// 替敲门的 Agent 进房间没成。它这时停用了就和没在敲门一样；别的都是暂时的，敲门还在等，房主过
/// 一会儿再放行一次就好。它的“令牌不对”别原样交出去：说的是敲门的那个，不是房主。
fn admission_failure(failure: NetworkGatewayFailure) -> NetworkGatewayFailure {
    match failure {
        NetworkGatewayFailure::Agent(failure)
            if failure.kind() == NetworkAgentFailureKind::Unauthorized =>
        {
            NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
                NetworkAgentFailureKind::KnockNotFound,
            ))
        }
        _ => NetworkGatewayFailure::Unavailable,
    }
}

impl NetworkAgentRooms for NetworkGateway {
    fn create_room<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentOwnedRoom, NetworkGatewayFailure>> {
        Box::pin(self.create_room_internal(token, name))
    }

    fn admit_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, Result<PrivateRoomAgentMemberRecord, NetworkGatewayFailure>> {
        Box::pin(self.admit_knock_internal(token, room, agent_id))
    }
}
