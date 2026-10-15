use agent_room_application::{
    persistence::RepositoryResult,
    ports::{AgentRoomKind, AgentRoomPlacement, AgentRoomPlacementStore, MatrixUserId, PortFuture},
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomCatalogStatus},
};
use sqlx::postgres::PgRow;

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

use super::decode::corrupt_data;

impl AgentRoomPlacementStore for PostgresRepositories {
    fn placements(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentRoomPlacement>>> {
        Box::pin(async move {
            let operation = "agent_room.placements";
            let rows = sqlx::query(
                r"SELECT reservation.id AS reservation_id,
                         reservation.agent_instance_id,
                         reservation.catalog_entry_id,
                         reservation.room_instance_id,
                         instance.matrix_room_id,
                         catalog.kind,
                         catalog.name,
                         catalog.status
                  FROM agent_room.room_capacity_reservation AS reservation
                  JOIN agent_room.room_instance AS instance
                    ON instance.id = reservation.room_instance_id
                  JOIN agent_room.room_catalog_entry AS catalog
                    ON catalog.id = reservation.catalog_entry_id
                  WHERE reservation.agent_id = $1
                    AND reservation.state = 'committed'
                    AND catalog.kind IN ('public_lobby', 'private_room')
                  ORDER BY reservation.reserved_at, reservation.id",
            )
            .bind(agent_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_placement(row, operation))
                .collect()
        })
    }

    fn matrix_user(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(async move {
            let operation = "agent_room.matrix_user";
            let user: Option<String> =
                sqlx::query_scalar("SELECT matrix_user_id FROM agent_room.agent WHERE id = $1")
                    .bind(agent_id.as_uuid())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
            user.map(MatrixUserId::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))
        })
    }

    fn forget_agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "agent_room.agent_member.forget";
            // 只删还算已加入的：被管理者移出的那条留着，它拿移出以前的口令照旧进不来。
            let result = sqlx::query(
                r"DELETE FROM agent_room.private_room_agent_member
                   WHERE catalog_entry_id = $1
                     AND agent_id = $2
                     AND membership_status = 'joined'",
            )
            .bind(catalog_id.as_uuid())
            .bind(agent_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(result.rows_affected() > 0)
        })
    }
}

fn decode_placement(row: &PgRow, operation: &'static str) -> RepositoryResult<AgentRoomPlacement> {
    let reservation_id: uuid::Uuid = decode_column(row, "reservation_id", operation)?;
    let agent_instance_id: uuid::Uuid = decode_column(row, "agent_instance_id", operation)?;
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let room_instance_id: uuid::Uuid = decode_column(row, "room_instance_id", operation)?;
    let matrix_room_id: String = decode_column(row, "matrix_room_id", operation)?;
    let kind: String = decode_column(row, "kind", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    Ok(AgentRoomPlacement {
        reservation_id: RoomReservationId::from_uuid(reservation_id),
        agent_instance_id: AgentInstanceId::from_uuid(agent_instance_id),
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        room_instance_id: RoomInstanceId::from_uuid(room_instance_id),
        matrix_room_id: MatrixRoomReference::new(matrix_room_id)
            .map_err(|_| corrupt_data(operation))?,
        kind: match kind.as_str() {
            "public_lobby" => AgentRoomKind::PublicLobby,
            "private_room" => AgentRoomKind::PrivateRoom,
            _ => return Err(corrupt_data(operation)),
        },
        name: decode_column(row, "name", operation)?,
        status: RoomCatalogStatus::try_from(status.as_str())
            .map_err(|_| corrupt_data(operation))?,
    })
}
