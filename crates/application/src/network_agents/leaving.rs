//! 网络 Agent 只离开一间（`specs/redesign/sub-rooms.md` 第 2h-3b 步）：`agent_room_leave` 带上
//! `roomId`，或者 `DELETE /v1/network-agents/me/rooms/{房间}`。不带的照旧全部离开、停用。
//!
//! 离开那一半和本机 Agent 离开一个房间是同一套（[`AgentRoomPlacementUseCases::leave`]，它的设备和
//! 连接对得上就行）：私人房间里先收回发言级别，以应用服务的身份让它在 Matrix 上离开，放掉占着的
//! 位置；凭口令或敲门进来的再删掉 Agent 成员记录，以后要重新凭口令或敲门。网络 Agent 这边再忘掉
//! 它进过的这间，和这间房里还没确认的消息、消息记录。
//!
//! 它自己建的房间也能离开：房间不关，有人敲门照样叫醒它，拿房间号随时回来。要关用
//! `agent_room_close_room`。

use agent_room_domain::{ids::RoomCatalogId, join_codes::parse_room_number};

use super::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentResult, NetworkAgentRoom,
    NetworkAgentService, dependency, internal, repository,
};
use crate::{
    agent_lobbies::AgentLobbyEntryFailure,
    agent_room_placements::{AgentRoomActor, LeaveAgentRoom},
    ports::NetworkAgentRoomRecord,
};

impl NetworkAgentService {
    pub(super) async fn leave_room_internal(
        &self,
        token: &str,
        room: &str,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        let record = self.authenticated(token).await?;
        let now = self.clock.now();
        self.store
            .record_activity(record.id, now)
            .await
            .map_err(repository)?;
        let wanted = parse_room_number(room);
        let matrix_room_id = room.trim();
        let matches = |catalog_id: RoomCatalogId, matrix: &str| {
            wanted.map_or(matrix == matrix_room_id, |wanted| wanted == catalog_id)
        };
        let joined = self
            .store
            .rooms(record.id)
            .await
            .map_err(repository)?
            .into_iter()
            .find(|joined| matches(joined.catalog_id, joined.matrix_room_id.as_str()));
        let left = match joined {
            Some(joined) => {
                self.placements
                    .leave(LeaveAgentRoom {
                        actor: AgentRoomActor::Device {
                            device: self.actor_of(&record, now)?,
                            agent_instance_id: record.agent_instance_id.ok_or_else(internal)?,
                        },
                        agent_id: record.agent_id.ok_or_else(internal)?,
                        catalog_id: joined.catalog_id,
                    })
                    .await
                    .map_err(|failure| placement_failure(&failure))?;
                joined
            }
            // 闲置时请出了公开大厅：Matrix 上早离开了，位置也放了，只剩“下次替它回去”那一笔。
            None => self
                .store
                .lobbies_away(record.id)
                .await
                .map_err(repository)?
                .into_iter()
                .find(|away| matches(away.catalog_id, away.matrix_room_id.as_str()))
                .map(|away| NetworkAgentRoomRecord {
                    catalog_id: away.catalog_id,
                    matrix_room_id: away.matrix_room_id,
                    joined_at: away.left_at,
                    name: None,
                    archived: false,
                    parent: None,
                })
                .ok_or_else(|| NetworkAgentFailure::new(NetworkAgentFailureKind::RoomNotJoined))?,
        };
        self.store
            .forget_room(record.id, left.catalog_id)
            .await
            .map_err(repository)?;
        self.named(left).await
    }
}

/// 离开没做成：聊天服务器、库一时不通的再来一次就好；连接对不上这类不该发生的照内部错误说。
fn placement_failure(failure: &AgentLobbyEntryFailure) -> NetworkAgentFailure {
    match failure {
        AgentLobbyEntryFailure::Access(_) | AgentLobbyEntryFailure::Membership(_) => dependency(),
        _ => internal(),
    }
}
