use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentCreationClaim, AgentCreationReservation, AgentCreationWorkflow, AgentRegistration,
        OutboxMessage, PortFuture, SecretDigest,
    },
};
use agent_room_domain::{
    agents::Agent,
    ids::{AgentCreationRequestId, AgentId, PrincipalId},
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_registered_agent, insert_agent_registration},
    error::map_sqlx_error,
    outbox::insert_outbox_event,
};

impl AgentCreationWorkflow for PostgresRepositories {
    fn reserve<'a>(
        &'a self,
        claim: &'a AgentCreationClaim,
    ) -> PortFuture<'a, RepositoryResult<AgentCreationReservation>> {
        Box::pin(async move { self.reserve_agent_creation(claim).await })
    }

    fn complete_with_event<'a>(
        &'a self,
        request_id: AgentCreationRequestId,
        request_fingerprint: &'a SecretDigest,
        registration: &'a AgentRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move {
            self.complete_agent_creation(request_id, request_fingerprint, registration, event)
                .await
        })
    }
}

impl PostgresRepositories {
    async fn reserve_agent_creation(
        &self,
        claim: &AgentCreationClaim,
    ) -> RepositoryResult<AgentCreationReservation> {
        let operation = "agent_creation.reserve";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            sqlx::query(
                r"INSERT INTO agent_room.agent_creation_request (
                    id, principal_id, agent_id, request_fingerprint, state, created_at
                ) VALUES (
                    $1, $2, $3, $4, 'reserved',
                    to_timestamp($5::double precision / 1000.0)
                )
                ON CONFLICT (id) DO NOTHING",
            )
            .bind(claim.request_id.as_uuid())
            .bind(claim.owner_id.as_uuid())
            .bind(claim.proposed_agent_id.as_uuid())
            .bind(claim.request_fingerprint.as_bytes().as_slice())
            .bind(claim.reserved_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

            let row = lock_creation_request(&mut transaction, claim.request_id, operation).await?;
            ensure_claim_matches(&row, claim.owner_id, &claim.request_fingerprint, operation)?;
            let agent_id = creation_agent_id(&row, operation)?;
            let state: String = decode_column(&row, "state", operation)?;

            match state.as_str() {
                "reserved" => Ok(AgentCreationReservation::Reserved { agent_id }),
                "completed" => {
                    let registration = load_registered_agent(
                        &mut transaction,
                        agent_id,
                        "agent_creation.reserve.completed",
                    )
                    .await?
                    .ok_or_else(|| corrupt_data("agent_creation.reserve.completed"))?;
                    Ok(AgentCreationReservation::Completed(registration))
                }
                _ => Err(corrupt_data(operation)),
            }
        }
        .await;

        crate::transaction::finish(transaction, result, operation).await
    }

    async fn complete_agent_creation(
        &self,
        request_id: AgentCreationRequestId,
        request_fingerprint: &SecretDigest,
        registration: &AgentRegistration,
        event: &OutboxMessage,
    ) -> RepositoryResult<Agent> {
        let operation = "agent_creation.complete";
        ensure_registration_event_contract(registration, event)?;
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            let row = lock_creation_request(&mut transaction, request_id, operation).await?;
            ensure_claim_matches(&row, registration.owner_id, request_fingerprint, operation)?;
            let reserved_agent_id = creation_agent_id(&row, operation)?;
            if reserved_agent_id != registration.agent.id() {
                return Err(conflict(operation));
            }
            let state: String = decode_column(&row, "state", operation)?;
            match state.as_str() {
                "completed" => {
                    let existing = load_registered_agent(
                        &mut transaction,
                        reserved_agent_id,
                        "agent_creation.complete.existing",
                    )
                    .await?
                    .ok_or_else(|| corrupt_data("agent_creation.complete.existing"))?;
                    Ok(existing.agent)
                }
                "reserved" => {
                    insert_agent_registration(&mut transaction, registration).await?;
                    insert_outbox_event(&mut transaction, event).await?;
                    let updated = sqlx::query(
                        r"UPDATE agent_room.agent_creation_request
                           SET state = 'completed',
                               completed_at = to_timestamp($2::double precision / 1000.0)
                           WHERE id = $1 AND state = 'reserved'",
                    )
                    .bind(request_id.as_uuid())
                    .bind(event.occurred_at().value())
                    .execute(&mut *transaction)
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
                    if updated.rows_affected() != 1 {
                        return Err(conflict(operation));
                    }
                    Ok(registration.agent.clone())
                }
                _ => Err(corrupt_data(operation)),
            }
        }
        .await;

        crate::transaction::finish(transaction, result, operation).await
    }
}

async fn lock_creation_request(
    transaction: &mut Transaction<'_, Postgres>,
    request_id: AgentCreationRequestId,
    operation: &'static str,
) -> RepositoryResult<PgRow> {
    sqlx::query(
        r"SELECT principal_id, agent_id, request_fingerprint, state
          FROM agent_room.agent_creation_request
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(request_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::NotFound))
}

fn ensure_claim_matches(
    row: &PgRow,
    principal_id: PrincipalId,
    request_fingerprint: &SecretDigest,
    operation: &'static str,
) -> RepositoryResult<()> {
    let stored_principal: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    if stored_principal != principal_id.as_uuid() {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ));
    }
    let stored_fingerprint: Vec<u8> = decode_column(row, "request_fingerprint", operation)?;
    if stored_fingerprint.as_slice() != request_fingerprint.as_bytes() {
        return Err(conflict(operation));
    }
    Ok(())
}

fn creation_agent_id(row: &PgRow, operation: &'static str) -> RepositoryResult<AgentId> {
    let id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    Ok(AgentId::from_uuid(id))
}

async fn load_registered_agent(
    transaction: &mut Transaction<'_, Postgres>,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<Option<agent_room_application::ports::RegisteredAgent>> {
    let row = sqlx::query(
        r"SELECT matrix_user_id, slug, display_name, description, avatar_content_id,
               visibility, lifecycle_state, version,
               floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms
          FROM agent_room.agent
          WHERE id = $1",
    )
    .bind(agent_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| decode_registered_agent(&row, agent_id, operation))
        .transpose()
}

fn ensure_registration_event_contract(
    registration: &AgentRegistration,
    event: &OutboxMessage,
) -> RepositoryResult<()> {
    if event.aggregate_type() == "agent"
        && event.aggregate_id() == registration.agent.id().as_uuid()
        && event.event_type() == "agent.registered.v1"
    {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "agent_creation.complete.event_contract",
            RepositoryErrorKind::Constraint,
        ))
    }
}

fn decode_column<T>(row: &PgRow, column: &str, operation: &'static str) -> RepositoryResult<T>
where
    for<'decode> T: sqlx::Decode<'decode, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}

fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
