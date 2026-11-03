//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：Agent 按管理授权动手时，台账
//! 里照旧记一条治理动作，`moderation_action_agent` 另记是哪个 Agent、按哪份授权、它写的理由，和动作同一个
//! 事务写下。认人也在这里：人的账号就是他，Agent 的账号认成它的主人。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentModerationAttribution, AgentModerationRecord, AgentModerationStore, MatrixUserId,
        ModerationActionReservationOutcome, ModerationSubject, PortFuture, RoomModerationGrant,
    },
};
use agent_room_domain::{
    ids::{AgentId, ModerationActionId, PrincipalId, RoomCatalogId, RoomModerationGrantId},
    moderation::{ModerationAction, ModerationAuditEvent},
    network_agents::NETWORK_AGENT_ISSUER,
    room_moderation_grants::AgentModerationNote,
    time::UtcMillis,
};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    error::{map_domain_error, map_sqlx_error},
    moderation::reserve_action_in_transaction,
    rooms::active_agent_grant,
    transaction,
};

#[derive(FromRow)]
struct SubjectRow {
    principal_id: Uuid,
    network_agent: bool,
}

#[derive(FromRow)]
struct AttributionRow {
    action_id: Uuid,
    agent_id: Uuid,
    grant_id: Uuid,
    note: String,
    display_name: String,
}

impl AgentModerationStore for PostgresRepositories {
    fn active_agent_grant(
        &self,
        room_catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomModerationGrant>>> {
        Box::pin(
            async move { active_agent_grant(self.pool(), room_catalog_id, agent_id, now).await },
        )
    }

    fn moderation_subject<'a>(
        &'a self,
        matrix_user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationSubject>>> {
        Box::pin(async move { moderation_subject(self, matrix_user_id).await })
    }

    fn reserve_agent_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
        attribution: &'a AgentModerationAttribution,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>> {
        Box::pin(async move {
            const OPERATION: &str = "agent_moderation.reserve_action";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            let result =
                reserve_with_attribution(&mut transaction, action, audit, attribution).await;
            transaction::finish(transaction, result, OPERATION).await
        })
    }

    fn agent_actions(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentModerationRecord>>> {
        Box::pin(async move { agent_actions(self, room_catalog_id).await })
    }
}

/// 人的账号（还在用、不是网络 Agent 的合成主体：那个账号从不进房间）就是他；不然看是不是 Agent 的账号，
/// 认成它没撤的主人，先成为主人的在前。网络 Agent 的主人是替它记账的合成主体。
async fn moderation_subject(
    repositories: &PostgresRepositories,
    matrix_user_id: &MatrixUserId,
) -> RepositoryResult<Option<ModerationSubject>> {
    let operation = "agent_moderation.subject";
    let row: Option<SubjectRow> = sqlx::query_as(
        r"SELECT principal_id, network_agent FROM (
              SELECT principal.id AS principal_id, false AS network_agent,
                     0 AS priority, principal.created_at AS since
                FROM agent_room.principal AS principal
               WHERE principal.matrix_user_id = $1
                 AND principal.status = 'active'
                 AND principal.oidc_issuer <> $2
              UNION ALL
              SELECT owner.id, owner.oidc_issuer = $2, 1, ownership.created_at
                FROM agent_room.agent AS agent
                JOIN agent_room.agent_ownership AS ownership
                  ON ownership.agent_id = agent.id
                 AND ownership.role = 'owner'
                 AND ownership.revoked_at IS NULL
                JOIN agent_room.principal AS owner ON owner.id = ownership.principal_id
               WHERE agent.matrix_user_id = $1
          ) AS subject
          ORDER BY priority ASC, since ASC, principal_id ASC
          LIMIT 1",
    )
    .bind(matrix_user_id.as_str())
    .bind(NETWORK_AGENT_ISSUER)
    .fetch_optional(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(row.map(|row| ModerationSubject {
        principal_id: PrincipalId::from_uuid(row.principal_id),
        network_agent: row.network_agent,
    }))
}

/// 预留动作、写审计，再记下是哪个 Agent 做的。同一个动作已经在了（重放）就交回它，不再记。
async fn reserve_with_attribution(
    transaction: &mut Transaction<'_, Postgres>,
    action: &ModerationAction,
    audit: &ModerationAuditEvent,
    attribution: &AgentModerationAttribution,
) -> RepositoryResult<ModerationActionReservationOutcome> {
    const OPERATION: &str = "agent_moderation.reserve_action";
    if attribution.action_id != action.id() {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Constraint,
        ));
    }
    let outcome = reserve_action_in_transaction(transaction, action, audit).await?;
    if matches!(outcome, ModerationActionReservationOutcome::Reserved(_)) {
        sqlx::query(
            r"INSERT INTO agent_room.moderation_action_agent (action_id, agent_id, grant_id, note)
              VALUES ($1, $2, $3, $4)",
        )
        .bind(attribution.action_id.as_uuid())
        .bind(attribution.agent_id.as_uuid())
        .bind(attribution.grant_id.as_uuid())
        .bind(attribution.note.as_str())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    }
    Ok(outcome)
}

/// 这个房间里 Agent 做的动作，后做的在前，和台账一样最多 200 条。
async fn agent_actions(
    repositories: &PostgresRepositories,
    room_catalog_id: RoomCatalogId,
) -> RepositoryResult<Vec<AgentModerationRecord>> {
    let operation = "agent_moderation.list";
    let rows: Vec<AttributionRow> = sqlx::query_as(
        r"SELECT attribution.action_id, attribution.agent_id, attribution.grant_id,
                 attribution.note, agent.display_name
            FROM agent_room.moderation_action_agent AS attribution
            JOIN agent_room.moderation_action AS action ON action.id = attribution.action_id
            JOIN agent_room.agent AS agent ON agent.id = attribution.agent_id
           WHERE action.room_catalog_id = $1
           ORDER BY action.starts_at DESC, action.id DESC
           LIMIT 200",
    )
    .bind(room_catalog_id.as_uuid())
    .fetch_all(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.into_iter()
        .map(|row| {
            Ok(AgentModerationRecord {
                attribution: AgentModerationAttribution {
                    action_id: ModerationActionId::from_uuid(row.action_id),
                    agent_id: AgentId::from_uuid(row.agent_id),
                    grant_id: RoomModerationGrantId::from_uuid(row.grant_id),
                    note: AgentModerationNote::parse(&row.note)
                        .map_err(|error| map_domain_error(operation, &error))?,
                },
                agent_display_name: row.display_name,
            })
        })
        .collect()
}
