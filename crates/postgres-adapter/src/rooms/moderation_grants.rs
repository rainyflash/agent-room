//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：管理授权一份一行。同一个
//! 房间、同一个 Agent 没撤的只有一份（唯一索引卡着），新给的在同一个事务里先把旧的记成撤了。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{ActiveRoomModerationGrant, PortFuture, RoomModerationGrant, RoomModerationGrantStore},
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomModerationGrantId},
    room_moderation_grants::{RoomModerationPower, RoomModerationPowers, RoomModerationRules},
    time::UtcMillis,
};
use sqlx::{FromRow, postgres::PgRow};
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    error::{map_domain_error, map_sqlx_error},
};

const GRANT_COLUMNS: &str = r"grant_row.id, grant_row.catalog_entry_id, grant_row.agent_id,
    grant_row.granted_by, grant_row.rules, grant_row.allowed_actions,
    floor(extract(epoch FROM grant_row.created_at) * 1000)::bigint AS created_at_ms,
    floor(extract(epoch FROM grant_row.expires_at) * 1000)::bigint AS expires_at_ms,
    floor(extract(epoch FROM grant_row.revoked_at) * 1000)::bigint AS revoked_at_ms";

#[derive(FromRow)]
struct GrantRow {
    id: Uuid,
    catalog_entry_id: Uuid,
    agent_id: Uuid,
    granted_by: Uuid,
    rules: String,
    allowed_actions: Vec<String>,
    created_at_ms: i64,
    expires_at_ms: i64,
    revoked_at_ms: Option<i64>,
}

impl GrantRow {
    fn decode(self, operation: &'static str) -> RepositoryResult<RoomModerationGrant> {
        let domain = |error| map_domain_error(operation, &error);
        let powers = self
            .allowed_actions
            .iter()
            .map(|action| RoomModerationPower::try_from(action.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .and_then(RoomModerationPowers::new)
            .map_err(domain)?;
        Ok(RoomModerationGrant {
            id: RoomModerationGrantId::from_uuid(self.id),
            catalog_id: RoomCatalogId::from_uuid(self.catalog_entry_id),
            agent_id: AgentId::from_uuid(self.agent_id),
            granted_by: PrincipalId::from_uuid(self.granted_by),
            rules: RoomModerationRules::parse(&self.rules).map_err(domain)?,
            powers,
            created_at: UtcMillis::new(self.created_at_ms).map_err(domain)?,
            expires_at: UtcMillis::new(self.expires_at_ms).map_err(domain)?,
            revoked_at: self
                .revoked_at_ms
                .map(UtcMillis::new)
                .transpose()
                .map_err(domain)?,
        })
    }
}

impl RoomModerationGrantStore for PostgresRepositories {
    fn owned_agent_name(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<String>>> {
        Box::pin(async move {
            let operation = "room_moderation_grant.owned_agent";
            sqlx::query_scalar(
                r"SELECT agent.display_name
                    FROM agent_room.agent AS agent
                    JOIN agent_room.agent_ownership AS ownership
                      ON ownership.agent_id = agent.id
                     AND ownership.principal_id = $1
                     AND ownership.role IN ('owner', 'operator')
                     AND ownership.revoked_at IS NULL
                   WHERE agent.id = $2 AND agent.lifecycle_state = 'active'",
            )
            .bind(principal_id.as_uuid())
            .bind(agent_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))
        })
    }

    fn replace_room_moderation_grant<'a>(
        &'a self,
        grant: &'a RoomModerationGrant,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "room_moderation_grant.replace";
            let map = |error| map_sqlx_error(operation, &error);
            let mut transaction = self.pool().begin().await.map_err(map)?;
            sqlx::query(
                r"UPDATE agent_room.room_moderation_grant
                     SET revoked_at = greatest(created_at, to_timestamp($4::double precision / 1000.0)),
                         revoked_by = $3
                   WHERE catalog_entry_id = $1 AND agent_id = $2 AND revoked_at IS NULL",
            )
            .bind(grant.catalog_id.as_uuid())
            .bind(grant.agent_id.as_uuid())
            .bind(grant.granted_by.as_uuid())
            .bind(grant.created_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(map)?;
            let actions: Vec<&str> = grant
                .powers
                .as_slice()
                .iter()
                .map(|power| power.as_str())
                .collect();
            sqlx::query(
                r"INSERT INTO agent_room.room_moderation_grant
                      (id, catalog_entry_id, agent_id, granted_by, rules, allowed_actions,
                       created_at, expires_at)
                  VALUES ($1, $2, $3, $4, $5, $6,
                          to_timestamp($7::double precision / 1000.0),
                          to_timestamp($8::double precision / 1000.0))",
            )
            .bind(grant.id.as_uuid())
            .bind(grant.catalog_id.as_uuid())
            .bind(grant.agent_id.as_uuid())
            .bind(grant.granted_by.as_uuid())
            .bind(grant.rules.as_str())
            .bind(&actions)
            .bind(grant.created_at.value())
            .bind(grant.expires_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(map)?;
            transaction.commit().await.map_err(map)
        })
    }

    fn active_room_moderation_grants(
        &self,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<ActiveRoomModerationGrant>>> {
        Box::pin(async move {
            let operation = "room_moderation_grant.active";
            let rows: Vec<PgRow> = sqlx::query(sqlx::AssertSqlSafe(format!(
                r"SELECT {GRANT_COLUMNS}, agent.display_name AS agent_display_name
                    FROM agent_room.room_moderation_grant AS grant_row
                    JOIN agent_room.agent AS agent ON agent.id = grant_row.agent_id
                   WHERE grant_row.catalog_entry_id = $1
                     AND grant_row.revoked_at IS NULL
                     AND grant_row.expires_at > to_timestamp($2::double precision / 1000.0)
                   ORDER BY grant_row.created_at, grant_row.id"
            )))
            .bind(catalog_id.as_uuid())
            .bind(now.value())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| {
                    let agent_display_name = sqlx::Row::try_get(row, "agent_display_name")
                        .map_err(|error| map_sqlx_error(operation, &error))?;
                    let grant = GrantRow::from_row(row)
                        .map_err(|error| map_sqlx_error(operation, &error))?
                        .decode(operation)?;
                    Ok(ActiveRoomModerationGrant {
                        grant,
                        agent_display_name,
                    })
                })
                .collect()
        })
    }

    fn room_moderation_grant(
        &self,
        id: RoomModerationGrantId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomModerationGrant>>> {
        Box::pin(async move {
            let operation = "room_moderation_grant.find";
            let row: Option<GrantRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                r"SELECT {GRANT_COLUMNS}
                    FROM agent_room.room_moderation_grant AS grant_row
                   WHERE grant_row.id = $1"
            )))
            .bind(id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| row.decode(operation)).transpose()
        })
    }

    fn revoke_room_moderation_grant(
        &self,
        id: RoomModerationGrantId,
        revoked_by: PrincipalId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "room_moderation_grant.revoke";
            let revoked = sqlx::query(
                r"UPDATE agent_room.room_moderation_grant
                     SET revoked_at = greatest(created_at, to_timestamp($3::double precision / 1000.0)),
                         revoked_by = $2
                   WHERE id = $1 AND revoked_at IS NULL",
            )
            .bind(id.as_uuid())
            .bind(revoked_by.as_uuid())
            .bind(revoked_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .rows_affected();
            Ok(revoked > 0)
        })
    }
}
