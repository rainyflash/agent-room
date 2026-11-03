//! Agent 解封：找出房间里还封着的，撤回时同一个事务记下是哪个 Agent 撤的。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{AgentModerationRecord, AgentModerationReversal, AppliedBan},
};
use agent_room_domain::{
    ids::{ModerationActionId, RoomCatalogId},
    moderation::{ModerationAction, ModerationAuditEvent},
};
use sqlx::{FromRow, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    error::map_sqlx_error,
    moderation::{ACTION_COLUMNS, decode_action, finalize_action_in_transaction},
};

use super::{AttributionRow, attribution, record};

#[derive(FromRow)]
struct ReversalRow {
    action_id: Uuid,
    request_id: Uuid,
    agent_id: Uuid,
    grant_id: Uuid,
    note: String,
}

/// 房间里已经落下、还没撤的封禁，先做的在前，带着是不是 Agent 做的。封禁不多，不设条数。
pub(super) async fn applied_bans(
    repositories: &PostgresRepositories,
    room_catalog_id: RoomCatalogId,
) -> RepositoryResult<Vec<AppliedBan>> {
    let operation = "agent_moderation.applied_bans";
    let statement = format!(
        "SELECT {ACTION_COLUMNS},
                attribution.action_id IS NOT NULL AS by_agent
           FROM agent_room.moderation_action
           LEFT JOIN agent_room.moderation_action_agent AS attribution
             ON attribution.action_id = moderation_action.id
          WHERE moderation_action.room_catalog_id = $1
            AND moderation_action.action_type = 'ban'
            AND moderation_action.status = 'applied'
          ORDER BY moderation_action.starts_at, moderation_action.id"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(room_catalog_id.as_uuid())
        .fetch_all(repositories.pool())
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| {
            Ok(AppliedBan {
                action: decode_action(row, operation)?,
                by_agent: row
                    .try_get("by_agent")
                    .map_err(|error| map_sqlx_error(operation, &error))?,
            })
        })
        .collect()
}

/// 把封禁记成撤回、写审计，再记下是哪个 Agent 撤的。已经撤了（重放）就交回它，不再记。
pub(super) async fn finalize_with_reversal(
    transaction: &mut Transaction<'_, Postgres>,
    action: &ModerationAction,
    audit: &ModerationAuditEvent,
    reversal: &AgentModerationReversal,
) -> RepositoryResult<ModerationAction> {
    const OPERATION: &str = "agent_moderation.finalize_reversal";
    if reversal.attribution.action_id != action.id() {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Constraint,
        ));
    }
    let reversed = finalize_action_in_transaction(transaction, action, audit).await?;
    sqlx::query(
        r"INSERT INTO agent_room.moderation_action_agent_reversal
              (action_id, request_id, agent_id, grant_id, note)
          VALUES ($1, $2, $3, $4, $5)
          ON CONFLICT (action_id) DO NOTHING",
    )
    .bind(reversal.attribution.action_id.as_uuid())
    .bind(reversal.request_id.as_uuid())
    .bind(reversal.attribution.agent_id.as_uuid())
    .bind(reversal.attribution.grant_id.as_uuid())
    .bind(reversal.attribution.note.as_str())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    Ok(reversed)
}

/// 这个解封请求撤回的封禁里最后撤的那条。
pub(super) async fn agent_reversal(
    repositories: &PostgresRepositories,
    request_id: ModerationActionId,
) -> RepositoryResult<Option<AgentModerationReversal>> {
    let operation = "agent_moderation.reversal";
    let row: Option<ReversalRow> = sqlx::query_as(
        r"SELECT reversal.action_id, reversal.request_id, reversal.agent_id, reversal.grant_id,
                 reversal.note
            FROM agent_room.moderation_action_agent_reversal AS reversal
            JOIN agent_room.moderation_action AS action ON action.id = reversal.action_id
           WHERE reversal.request_id = $1
           ORDER BY action.reversed_at DESC NULLS LAST, action.id DESC
           LIMIT 1",
    )
    .bind(request_id.as_uuid())
    .fetch_optional(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| {
        Ok(AgentModerationReversal {
            request_id: ModerationActionId::from_uuid(row.request_id),
            attribution: attribution(
                row.action_id,
                row.agent_id,
                row.grant_id,
                &row.note,
                operation,
            )?,
        })
    })
    .transpose()
}

/// 这个房间里 Agent 撤回的动作，后撤的在前，和台账一样最多 200 条。
pub(super) async fn agent_reversals(
    repositories: &PostgresRepositories,
    room_catalog_id: RoomCatalogId,
) -> RepositoryResult<Vec<AgentModerationRecord>> {
    let operation = "agent_moderation.list_reversals";
    let rows: Vec<AttributionRow> = sqlx::query_as(
        r"SELECT reversal.action_id, reversal.agent_id, reversal.grant_id, reversal.note,
                 agent.display_name
            FROM agent_room.moderation_action_agent_reversal AS reversal
            JOIN agent_room.moderation_action AS action ON action.id = reversal.action_id
            JOIN agent_room.agent AS agent ON agent.id = reversal.agent_id
           WHERE action.room_catalog_id = $1
           ORDER BY action.reversed_at DESC NULLS LAST, action.id DESC
           LIMIT 200",
    )
    .bind(room_catalog_id.as_uuid())
    .fetch_all(repositories.pool())
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.into_iter().map(|row| record(row, operation)).collect()
}
