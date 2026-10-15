use agent_room_application::{
    persistence::RepositoryResult,
    ports::{PortFuture, RoomAgentKind, RoomAgentOwnerRecord, RoomPeopleDirectory, RoomPersonName},
};
use agent_room_domain::ids::{AgentId, PrincipalId, RoomCatalogId};
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

impl RoomPeopleDirectory for PostgresRepositories {
    fn member_names(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomPersonName>>> {
        Box::pin(async move {
            let operation = "room_people.member_names";
            let rows = sqlx::query(
                r"SELECT member.principal_id, principal.display_name
                  FROM agent_room.private_room_membership AS member
                  JOIN agent_room.principal AS principal ON principal.id = member.principal_id
                  WHERE member.catalog_entry_id = $1
                  ORDER BY member.created_at, member.principal_id",
            )
            .bind(catalog_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| {
                    person(row, "principal_id", "display_name", operation)?
                        .ok_or_else(|| super::decode::corrupt_data(operation))
                })
                .collect()
        })
    }

    fn agent_owners<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        agent_ids: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<RoomAgentOwnerRecord>>> {
        Box::pin(async move {
            let operation = "room_people.agent_owners";
            let candidates: Vec<Uuid> = agent_ids.iter().map(|id| id.as_uuid()).collect();
            // 在房间里：本机 Agent 有已确认的位置，网络 Agent 记着在这个房间。主人取最早的那位还有效
            // 的所有者；网络 Agent 的所有者是替它记账的合成主体，不当人类主人。
            let rows = sqlx::query(
                r"WITH present AS (
                      SELECT reservation.agent_id
                        FROM agent_room.room_capacity_reservation AS reservation
                       WHERE reservation.catalog_entry_id = $1
                         AND reservation.state = 'committed'
                         AND reservation.agent_id = ANY($2)
                      UNION
                      SELECT network.agent_id
                        FROM agent_room.network_agent_room AS room
                        JOIN agent_room.network_agent AS network
                          ON network.id = room.network_agent_id
                       WHERE room.catalog_entry_id = $1
                         AND network.agent_id = ANY($2)
                  )
                  SELECT present.agent_id,
                         network.id IS NOT NULL AS is_network,
                         owner.principal_id AS owner_principal_id,
                         owner_principal.display_name AS owner_display_name,
                         knock.decided_by_principal_id AS admitted_by_principal_id,
                         admitter.display_name AS admitted_by_display_name
                    FROM present
                    LEFT JOIN agent_room.network_agent AS network
                      ON network.agent_id = present.agent_id
                    LEFT JOIN LATERAL (
                        SELECT ownership.principal_id
                          FROM agent_room.agent_ownership AS ownership
                         WHERE ownership.agent_id = present.agent_id
                           AND ownership.role = 'owner'
                           AND ownership.revoked_at IS NULL
                         ORDER BY ownership.created_at, ownership.principal_id
                         LIMIT 1
                    ) AS owner ON network.id IS NULL
                    LEFT JOIN agent_room.principal AS owner_principal
                      ON owner_principal.id = owner.principal_id
                    LEFT JOIN agent_room.private_room_agent_member AS member
                      ON member.catalog_entry_id = $1
                     AND member.agent_id = present.agent_id
                     AND member.membership_status = 'joined'
                     AND member.joined_via = 'knock'
                    LEFT JOIN agent_room.private_room_agent_knock AS knock
                      ON member.agent_id IS NOT NULL
                     AND knock.catalog_entry_id = $1
                     AND knock.agent_id = present.agent_id
                     AND knock.knock_status = 'admitted'
                    LEFT JOIN agent_room.principal AS admitter
                      ON admitter.id = knock.decided_by_principal_id
                   ORDER BY present.agent_id",
            )
            .bind(catalog_id.as_uuid())
            .bind(&candidates)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_owner(row, operation))
                .collect()
        })
    }
}

fn decode_owner(row: &PgRow, operation: &'static str) -> RepositoryResult<RoomAgentOwnerRecord> {
    let agent_id: Uuid = decode_column(row, "agent_id", operation)?;
    let is_network: bool = decode_column(row, "is_network", operation)?;
    Ok(RoomAgentOwnerRecord {
        agent_id: AgentId::from_uuid(agent_id),
        kind: if is_network {
            RoomAgentKind::Network
        } else {
            RoomAgentKind::Local
        },
        owner: person(row, "owner_principal_id", "owner_display_name", operation)?,
        admitted_by: person(
            row,
            "admitted_by_principal_id",
            "admitted_by_display_name",
            operation,
        )?,
    })
}

fn person(
    row: &PgRow,
    id_column: &'static str,
    name_column: &'static str,
    operation: &'static str,
) -> RepositoryResult<Option<RoomPersonName>> {
    let id: Option<Uuid> = decode_column(row, id_column, operation)?;
    let name: Option<String> = decode_column(row, name_column, operation)?;
    Ok(id.zip(name).map(|(id, display_name)| RoomPersonName {
        principal_id: PrincipalId::from_uuid(id),
        display_name,
    }))
}
