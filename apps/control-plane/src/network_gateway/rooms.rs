//! 网络 Agent 自己建房间、关房间（`specs/agent-rooms/design.md` 第 4 步）。建好以后和凭口令进私人
//! 房间一样：先让加密客户端就绪、建好加密身份，再进去，进了说一声在线。

use agent_room_application::{network_agents::NetworkAgentOwnedRoom, ports::PortFuture};

use super::{NetworkGateway, NetworkGatewayFailure};

/// 网络 Agent 管自己建的房间；HTTP 接口与远程 MCP 只认这个，便于单独测试。
pub(crate) trait NetworkAgentRooms: Send + Sync {
    /// 建房并进去；它没关的房间里已经有同名的，就回到那间（还没进去的这次进去）。
    fn create_room<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentOwnedRoom, NetworkGatewayFailure>>;

    /// 关掉自己建的房间；已经关了的也算成功。它照旧留在房间里，之前的消息还看得到。
    fn close_room<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>>;
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
}

impl NetworkAgentRooms for NetworkGateway {
    fn create_room<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
    ) -> PortFuture<'a, Result<NetworkAgentOwnedRoom, NetworkGatewayFailure>> {
        Box::pin(self.create_room_internal(token, name))
    }

    fn close_room<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        Box::pin(async move {
            self.owned_rooms
                .close_room(token, room)
                .await
                .map_err(NetworkGatewayFailure::Agent)
        })
    }
}
