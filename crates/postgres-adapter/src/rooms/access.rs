use agent_room_application::{
    persistence::RepositoryResult,
    ports::{AgentLobbyAccessRecord, AgentLobbyAccessRepository, MatrixUserId, PortFuture},
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId, RoomCatalogId, RoomInstanceId},
    rooms::MatrixRoomReference,
};
use sqlx::{Row, postgres::PgRow};

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

impl AgentLobbyAccessRepository for PostgresRepositories {
    fn find_public_lobby_room<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomInstanceId>>> {
        Box::pin(async move {
            let operation = "agent_lobby.room.find";
            let id: Option<uuid::Uuid> = sqlx::query_scalar(
                r"SELECT instance.id FROM agent_room.room_instance AS instance
                  JOIN agent_room.room_catalog_entry AS catalog ON catalog.id = instance.catalog_entry_id
                  WHERE catalog.id = $1 AND instance.matrix_room_id = $2
                    AND instance.state = 'active' AND catalog.status = 'active'
                    AND catalog.kind = 'public_lobby' AND catalog.visibility = 'public'",
            )
            .bind(catalog_id.as_uuid())
            .bind(matrix_room_id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(id.map(RoomInstanceId::from_uuid))
        })
    }

    fn find_lobby_access(
        &self,
        agent_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentLobbyAccessRecord>>> {
        Box::pin(async move {
            let operation = "agent_lobby.access.find";
            let row = sqlx::query(
                r"SELECT instance.id AS agent_instance_id,
                         instance.agent_id,
                         instance.device_id,
                         principal.id AS principal_id,
                         agent.matrix_user_id,
                         (
                             instance.revoked_at IS NULL
                             AND device.revoked_at IS NULL
                             AND device.trust_state = 'verified'
                             AND principal.status = 'active'
                             AND agent.lifecycle_state = 'active'
                             AND binding.state = 'active'
                         ) AS active
                  FROM agent_room.agent_instance AS instance
                  JOIN agent_room.device AS device ON device.id = instance.device_id
                  JOIN agent_room.principal AS principal ON principal.id = device.principal_id
                  JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
                  JOIN agent_room.adapter_binding AS binding
                    ON binding.id = instance.adapter_binding_id
                  WHERE instance.id = $1",
            )
            .bind(agent_instance_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.as_ref()
                .map(|row| decode_access(row, operation))
                .transpose()
        })
    }
}

fn decode_access(row: &PgRow, operation: &'static str) -> RepositoryResult<AgentLobbyAccessRecord> {
    let instance_id: uuid::Uuid = decode_column(row, "agent_instance_id", operation)?;
    let agent_id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    let device_id: uuid::Uuid = decode_column(row, "device_id", operation)?;
    let principal_id: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    let matrix_user_id: String = decode_column(row, "matrix_user_id", operation)?;
    let active: bool = row
        .try_get("active")
        .map_err(|_| super::decode::corrupt_data(operation))?;
    Ok(AgentLobbyAccessRecord {
        agent_id: AgentId::from_uuid(agent_id),
        agent_instance_id: AgentInstanceId::from_uuid(instance_id),
        device_id: DeviceId::from_uuid(device_id),
        principal_id: PrincipalId::from_uuid(principal_id),
        matrix_user_id: MatrixUserId::new(matrix_user_id)
            .map_err(|_| super::decode::corrupt_data(operation))?,
        active,
    })
}
