use agent_room_application::{
    persistence::RepositoryResult,
    ports::{AgentRetirementOutcome, AgentRetirementTransaction, OutboxMessage, PortFuture},
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId},
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction};

use crate::{PostgresRepositories, error::map_sqlx_error, outbox::insert_outbox_event};

const OPERATION: &str = "agent.retire";

impl AgentRetirementTransaction for PostgresRepositories {
    fn retire<'a>(
        &'a self,
        principal_id: PrincipalId,
        agent_id: AgentId,
        retired_at: UtcMillis,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentRetirementOutcome>> {
        Box::pin(async move {
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            let result =
                retire_in_transaction(&mut transaction, principal_id, agent_id, retired_at, event)
                    .await;
            crate::transaction::finish(transaction, result, OPERATION).await
        })
    }
}

async fn retire_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    agent_id: AgentId,
    retired_at: UtcMillis,
    event: &OutboxMessage,
) -> RepositoryResult<AgentRetirementOutcome> {
    // Locking the agent row serializes deletion with instance registration and membership changes.
    let state: Option<String> =
        sqlx::query_scalar("SELECT lifecycle_state FROM agent_room.agent WHERE id = $1 FOR UPDATE")
            .bind(agent_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let Some(state) = state else {
        return Ok(AgentRetirementOutcome::NotFound);
    };
    let (role, owners): (Option<String>, i64) = sqlx::query_as(
        r"SELECT
              (SELECT role FROM agent_room.agent_ownership
                WHERE agent_id = $1 AND principal_id = $2 AND revoked_at IS NULL),
              (SELECT count(*) FROM agent_room.agent_ownership
                WHERE agent_id = $1 AND role = 'owner' AND revoked_at IS NULL)",
    )
    .bind(agent_id.as_uuid())
    .bind(principal_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    match role.as_deref() {
        None => return Ok(AgentRetirementOutcome::NotFound),
        Some("owner") => {}
        Some(_) => return Ok(AgentRetirementOutcome::NotOwner),
    }
    if state == "retired" {
        return Ok(AgentRetirementOutcome::AlreadyRetired);
    }
    if owners > 1 {
        return Ok(AgentRetirementOutcome::NotSoleOwner);
    }
    let live_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.agent_instance WHERE agent_id = $1 AND revoked_at IS NULL",
    )
    .bind(agent_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if live_instances > 0 {
        return Ok(AgentRetirementOutcome::ActiveInstances);
    }

    sqlx::query(
        r"UPDATE agent_room.automation_grant
          SET state = 'revoked',
              revoked_at = to_timestamp($2::double precision / 1000.0),
              version = version + 1
          WHERE agent_id = $1 AND state = 'active' AND revoked_at IS NULL",
    )
    .bind(agent_id.as_uuid())
    .bind(retired_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    // Same anonymization as account deletion: the retired agent keeps no profile text.
    sqlx::query(
        r"UPDATE agent_room.agent
          SET lifecycle_state = 'retired', display_name = 'Deleted agent', description = '',
              avatar_content_id = NULL, visibility = 'private',
              updated_at = to_timestamp($2::double precision / 1000.0), version = version + 1
          WHERE id = $1",
    )
    .bind(agent_id.as_uuid())
    .bind(retired_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    insert_outbox_event(transaction, event).await?;
    Ok(AgentRetirementOutcome::Retired)
}
