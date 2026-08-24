use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentMembershipChange, AgentMembershipRepository, AgentMembershipTransaction,
        OutboxMessage, PortFuture,
    },
};
use agent_room_domain::{
    DomainError,
    agents::{AgentMember, AgentMemberships, AgentRole},
    ids::{AgentId, PrincipalId},
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_column, decode_optional_time, decode_time},
    error::map_sqlx_error,
    outbox::insert_outbox_event,
};

impl AgentMembershipRepository for PostgresRepositories {
    fn find_memberships(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentMemberships>>> {
        Box::pin(async move {
            let status: Option<String> =
                sqlx::query_scalar("SELECT lifecycle_state FROM agent_room.agent WHERE id = $1")
                    .bind(agent_id.as_uuid())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|error| map_sqlx_error("agent_membership.find", &error))?;
            if !status.is_some_and(|status| status == "active") {
                return Ok(None);
            }

            let rows = load_membership_rows(self.pool(), agent_id, "agent_membership.find").await?;
            decode_memberships(agent_id, &rows, "agent_membership.find").map(Some)
        })
    }
}

impl AgentMembershipTransaction for PostgresRepositories {
    fn apply_change<'a>(
        &'a self,
        change: &'a AgentMembershipChange,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentMemberships>> {
        Box::pin(async move { self.apply_membership_change(change, event).await })
    }
}

impl PostgresRepositories {
    async fn apply_membership_change(
        &self,
        change: &AgentMembershipChange,
        event: &OutboxMessage,
    ) -> RepositoryResult<AgentMemberships> {
        let operation = "agent_membership.change";
        ensure_event_contract(change, event)?;
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            lock_active_agent(&mut transaction, change.agent_id, operation).await?;
            let rows = load_membership_rows(&mut *transaction, change.agent_id, operation).await?;
            let mut memberships = decode_memberships(change.agent_id, &rows, operation)?;
            let before = memberships.clone();
            apply_domain_change(&mut memberships, change)
                .map_err(|error| map_domain_error(operation, &error))?;
            if memberships == before {
                return Ok(memberships);
            }

            persist_change(&mut transaction, change).await?;
            insert_outbox_event(&mut transaction, event).await?;
            Ok(memberships)
        }
        .await;

        crate::transaction::finish(transaction, result, operation).await
    }
}

fn apply_domain_change(
    memberships: &mut AgentMemberships,
    change: &AgentMembershipChange,
) -> Result<(), DomainError> {
    match change.role {
        Some(role) => memberships.grant_role(
            change.actor_id,
            change.principal_id,
            role,
            change.changed_at,
        ),
        None => memberships.revoke(change.actor_id, change.principal_id, change.changed_at),
    }
}

async fn persist_change(
    transaction: &mut Transaction<'_, Postgres>,
    change: &AgentMembershipChange,
) -> RepositoryResult<()> {
    match change.role {
        Some(role) => persist_grant(transaction, change, role).await,
        None => persist_revocation(transaction, change).await,
    }
}

async fn persist_grant(
    transaction: &mut Transaction<'_, Postgres>,
    change: &AgentMembershipChange,
    role: AgentRole,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
            principal_id, agent_id, role, granted_by, created_at, revoked_at
        ) VALUES (
            $1, $2, $3, $4,
            to_timestamp($5::double precision / 1000.0), NULL
        )
        ON CONFLICT (principal_id, agent_id) DO UPDATE
        SET role = EXCLUDED.role,
            granted_by = EXCLUDED.granted_by,
            created_at = EXCLUDED.created_at,
            revoked_at = NULL",
    )
    .bind(change.principal_id.as_uuid())
    .bind(change.agent_id.as_uuid())
    .bind(role.as_str())
    .bind(change.actor_id.as_uuid())
    .bind(change.changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_membership.grant", &error))?;
    Ok(())
}

async fn persist_revocation(
    transaction: &mut Transaction<'_, Postgres>,
    change: &AgentMembershipChange,
) -> RepositoryResult<()> {
    let updated = sqlx::query(
        r"UPDATE agent_room.agent_ownership
           SET revoked_at = to_timestamp($3::double precision / 1000.0)
           WHERE principal_id = $1 AND agent_id = $2 AND revoked_at IS NULL",
    )
    .bind(change.principal_id.as_uuid())
    .bind(change.agent_id.as_uuid())
    .bind(change.changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_membership.revoke", &error))?;
    if updated.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "agent_membership.revoke",
            RepositoryErrorKind::Conflict,
        ))
    }
}

async fn lock_active_agent(
    transaction: &mut Transaction<'_, Postgres>,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<()> {
    let state: Option<String> =
        sqlx::query_scalar("SELECT lifecycle_state FROM agent_room.agent WHERE id = $1 FOR UPDATE")
            .bind(agent_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
    match state.as_deref() {
        Some("active") => Ok(()),
        Some(_) => Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        )),
        None => Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::NotFound,
        )),
    }
}

async fn load_membership_rows<'executor, E>(
    executor: E,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<Vec<PgRow>>
where
    E: sqlx::Executor<'executor, Database = Postgres>,
{
    sqlx::query(
        r"SELECT principal_id, role, granted_by,
               floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms,
               floor(extract(epoch FROM revoked_at) * 1000)::bigint AS revoked_at_ms
          FROM agent_room.agent_ownership
          WHERE agent_id = $1
          ORDER BY created_at, principal_id
          FOR UPDATE",
    )
    .bind(agent_id.as_uuid())
    .fetch_all(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))
}

fn decode_memberships(
    agent_id: AgentId,
    rows: &[PgRow],
    operation: &'static str,
) -> RepositoryResult<AgentMemberships> {
    let members = rows
        .iter()
        .map(|row| decode_member(row, agent_id, operation))
        .collect::<RepositoryResult<Vec<_>>>()?;
    AgentMemberships::restore(agent_id, members)
        .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))
}

fn decode_member(
    row: &PgRow,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<AgentMember> {
    let principal_id: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    let granted_by: uuid::Uuid = decode_column(row, "granted_by", operation)?;
    let role: String = decode_column(row, "role", operation)?;
    let role = AgentRole::try_from(role.as_str())
        .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))?;
    AgentMember::restore(
        agent_id,
        PrincipalId::from_uuid(principal_id),
        role,
        PrincipalId::from_uuid(granted_by),
        decode_time(row, "created_at_ms", operation)?,
        decode_optional_time(row, "revoked_at_ms", operation)?,
    )
    .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))
}

fn ensure_event_contract(
    change: &AgentMembershipChange,
    event: &OutboxMessage,
) -> RepositoryResult<()> {
    if event.aggregate_type() == "agent"
        && event.aggregate_id() == change.agent_id.as_uuid()
        && event.event_type() == "agent.membership.changed.v1"
    {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "agent_membership.change.event_contract",
            RepositoryErrorKind::Constraint,
        ))
    }
}

fn map_domain_error(operation: &'static str, error: &DomainError) -> RepositoryError {
    let kind = match error {
        DomainError::Forbidden { .. } => RepositoryErrorKind::Forbidden,
        DomainError::InvariantViolation { .. }
        | DomainError::InvalidTransition { .. }
        | DomainError::Validation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => RepositoryErrorKind::Constraint,
    };
    RepositoryError::new(operation, kind)
}
