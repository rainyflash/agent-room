//! 一小时没动静的网络 Agent 先请出公开大厅（`specs/launch-capacity/design.md` 第 2 步）：
//! 定时清理找出它们、替它们离开以后记一笔，放回位置；它下次调接口时服务器替它回来，
//! 再进这个大厅时（`record_network_agent_room`）那一笔删掉。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{NetworkAgentIdleLobby, NetworkAgentLobbyAway, NetworkAgentRoomRecord},
};
use agent_room_domain::{
    ids::{NetworkAgentId, RoomCatalogId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};

use crate::{
    PostgresRepositories,
    error::{map_domain_error, map_sqlx_error},
};

impl PostgresRepositories {
    pub(crate) async fn idle_network_agents_in_lobbies(
        &self,
        idle_before: UtcMillis,
        limit: u32,
    ) -> RepositoryResult<Vec<NetworkAgentIdleLobby>> {
        let operation = "network_agent.idle_in_lobbies";
        let rows = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, i64, String)>(
            r"SELECT agent.id, room.catalog_entry_id, room.matrix_room_id,
                     floor(extract(epoch FROM room.joined_at) * 1000)::bigint,
                     catalog.name
                FROM agent_room.network_agent agent
                JOIN agent_room.network_agent_room room ON room.network_agent_id = agent.id
                JOIN agent_room.room_catalog_entry catalog ON catalog.id = room.catalog_entry_id
               WHERE agent.status = 'active'
                 AND agent.last_active_at < to_timestamp($1::double precision / 1000.0)
                 AND catalog.kind = 'public_lobby'
               ORDER BY agent.last_active_at, agent.id, room.matrix_room_id
               LIMIT $2",
        )
        .bind(idle_before.value())
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        rows.into_iter()
            .map(|(id, catalog_id, matrix_room_id, joined_at, name)| {
                Ok(NetworkAgentIdleLobby {
                    id: NetworkAgentId::from_uuid(id),
                    room: NetworkAgentRoomRecord {
                        catalog_id: RoomCatalogId::from_uuid(catalog_id),
                        matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                            .map_err(|error| map_domain_error(operation, &error))?,
                        joined_at: UtcMillis::new(joined_at)
                            .map_err(|error| map_domain_error(operation, &error))?,
                        name: Some(name),
                    },
                })
            })
            .collect()
    }

    /// 一条语句做完：去掉进过的这个房间，记下离开的大厅和分片，放回它在这个分片占的位置。
    /// 位置照 Matrix 的事实放：已经离开了这个分片，不管库里记没记着它在。先改预约、再改分片，
    /// 和分配时的加锁顺序一样。
    pub(crate) async fn mark_network_agent_lobby_away(
        &self,
        id: NetworkAgentId,
        room: &NetworkAgentRoomRecord,
        at: UtcMillis,
    ) -> RepositoryResult<()> {
        let operation = "network_agent.mark_lobby_away";
        sqlx::query(
            r"WITH gone AS (
                  DELETE FROM agent_room.network_agent_room room
                   WHERE room.network_agent_id = $1
                     AND room.matrix_room_id = $2
                     AND room.joined_at <= to_timestamp($4::double precision / 1000.0)
               RETURNING room.catalog_entry_id, room.matrix_room_id
              ),
              away AS (
                  INSERT INTO agent_room.network_agent_lobby_away
                         (network_agent_id, catalog_entry_id, matrix_room_id, left_at)
                  SELECT $1, gone.catalog_entry_id, gone.matrix_room_id,
                         to_timestamp($3::double precision / 1000.0)
                    FROM gone
                  ON CONFLICT (network_agent_id, catalog_entry_id) DO UPDATE
                     SET matrix_room_id = EXCLUDED.matrix_room_id,
                         left_at = EXCLUDED.left_at
              ),
              released AS (
                  UPDATE agent_room.room_capacity_reservation reservation
                     SET state = 'released',
                         finalized_at = greatest(reservation.reserved_at,
                             to_timestamp($3::double precision / 1000.0))
                    FROM agent_room.network_agent agent, agent_room.room_instance instance
                   WHERE agent.id = $1
                     AND reservation.agent_instance_id = agent.agent_instance_id
                     AND reservation.room_instance_id = instance.id
                     AND instance.matrix_room_id = $2
                     AND reservation.state IN ('reserved', 'committed')
                     AND NOT EXISTS (
                         SELECT 1
                           FROM agent_room.network_agent_room current_room
                          WHERE current_room.network_agent_id = $1
                            AND current_room.matrix_room_id = $2
                            AND current_room.joined_at > to_timestamp($4::double precision / 1000.0)
                     )
               RETURNING reservation.room_instance_id
              )
              UPDATE agent_room.room_instance instance
                 SET allocated_slots = greatest(instance.allocated_slots - counted.slots, 0),
                     updated_at = greatest(instance.updated_at,
                         to_timestamp($3::double precision / 1000.0)),
                     version = instance.version + 1
                FROM (SELECT room_instance_id, count(*)::integer AS slots
                        FROM released GROUP BY room_instance_id) counted
               WHERE instance.id = counted.room_instance_id",
        )
        .bind(id.as_uuid())
        .bind(room.matrix_room_id.as_str())
        .bind(at.value())
        .bind(room.joined_at.value())
        .execute(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        Ok(())
    }

    pub(crate) async fn network_agent_lobbies_away(
        &self,
        id: NetworkAgentId,
    ) -> RepositoryResult<Vec<NetworkAgentLobbyAway>> {
        let operation = "network_agent.lobbies_away";
        let rows = sqlx::query_as::<_, (uuid::Uuid, String, i64)>(
            r"SELECT catalog_entry_id, matrix_room_id,
                     floor(extract(epoch FROM left_at) * 1000)::bigint
                FROM agent_room.network_agent_lobby_away
               WHERE network_agent_id = $1
               ORDER BY left_at, catalog_entry_id",
        )
        .bind(id.as_uuid())
        .fetch_all(self.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
        rows.into_iter()
            .map(|(catalog_id, matrix_room_id, left_at)| {
                Ok(NetworkAgentLobbyAway {
                    catalog_id: RoomCatalogId::from_uuid(catalog_id),
                    matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                        .map_err(|error| map_domain_error(operation, &error))?,
                    left_at: UtcMillis::new(left_at)
                        .map_err(|error| map_domain_error(operation, &error))?,
                })
            })
            .collect()
    }
}
