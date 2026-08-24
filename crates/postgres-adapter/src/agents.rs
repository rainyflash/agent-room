use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentRegistration, AgentRegistrationTransaction, AgentRepository, OutboxMessage,
        PortFuture, RegisteredAgent,
    },
};
use agent_room_domain::{
    agents::{Agent, AgentStatus, AgentVisibility},
    ids::{AgentId, ContentId},
    time::UtcMillis,
    version::AggregateVersion,
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use crate::{PostgresRepositories, error::map_sqlx_error, outbox::insert_outbox_event};

impl AgentRepository for PostgresRepositories {
    fn find(&self, id: AgentId) -> PortFuture<'_, RepositoryResult<Option<Agent>>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"SELECT lifecycle_state, version
                  FROM agent_room.agent
                  WHERE id = $1",
            )
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("agent.find", &error))?;

            row.map(decode_agent(id)).transpose()
        })
    }

    fn find_registration(
        &self,
        id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<RegisteredAgent>>> {
        Box::pin(
            async move { find_registered_agent(&self.pool, id, "agent.find_registration").await },
        )
    }

    fn create<'a>(
        &'a self,
        registration: &'a AgentRegistration,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move { self.create_in_transaction(registration, None).await })
    }

    fn save<'a>(&'a self, agent: &'a Agent) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move {
            let next_version = agent
                .version()
                .next()
                .map_err(|_| corrupt_data("agent.save"))?;
            let row = sqlx::query(
                r"UPDATE agent_room.agent
                   SET lifecycle_state = $2, updated_at = clock_timestamp(), version = $3
                   WHERE id = $1 AND version = $4
                   RETURNING version",
            )
            .bind(agent.id().as_uuid())
            .bind(agent.status().as_str())
            .bind(next_version.value())
            .bind(agent.version().value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("agent.save", &error))?;

            match row {
                Some(row) => {
                    let version = decode_version(&row, "agent.save")?;
                    Ok(Agent::restore(agent.id(), agent.status(), version))
                }
                None => Err(classify_missing_agent(&self.pool, agent.id()).await?),
            }
        })
    }
}

impl AgentRegistrationTransaction for PostgresRepositories {
    fn create_with_event<'a>(
        &'a self,
        registration: &'a AgentRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move { self.create_in_transaction(registration, Some(event)).await })
    }
}

impl PostgresRepositories {
    async fn create_in_transaction(
        &self,
        registration: &AgentRegistration,
        event: Option<&OutboxMessage>,
    ) -> RepositoryResult<Agent> {
        if event.is_some_and(|event| {
            event.aggregate_type() != "agent"
                || event.aggregate_id() != registration.agent.id().as_uuid()
                || event.event_type() != "agent.registered.v1"
        }) {
            return Err(RepositoryError::new(
                "agent.create.event_contract",
                RepositoryErrorKind::Constraint,
            ));
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error("agent.create.begin", &error))?;
        let result = async {
            insert_agent_registration(&mut transaction, registration).await?;
            if let Some(event) = event {
                insert_outbox_event(&mut transaction, event).await?;
            }
            Ok(())
        }
        .await;

        if let Err(error) = result {
            transaction
                .rollback()
                .await
                .map_err(|rollback| map_sqlx_error("agent.create.rollback", &rollback))?;
            return Err(error);
        }

        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error("agent.create.commit", &error))?;
        Ok(registration.agent.clone())
    }
}

pub(crate) async fn insert_agent_registration(
    transaction: &mut Transaction<'_, Postgres>,
    registration: &AgentRegistration,
) -> RepositoryResult<()> {
    let agent = &registration.agent;
    let avatar_id = registration.avatar_content_id.map(ContentId::as_uuid);
    let registered_at = registration.registered_at.value();

    sqlx::query(
        r"INSERT INTO agent_room.agent (
            id, matrix_user_id, slug, display_name, description, avatar_content_id,
            visibility, lifecycle_state, created_at, updated_at, version
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            to_timestamp($9::double precision / 1000.0),
            to_timestamp($9::double precision / 1000.0), $10
        )",
    )
    .bind(agent.id().as_uuid())
    .bind(&registration.matrix_user_id)
    .bind(&registration.slug)
    .bind(&registration.display_name)
    .bind(&registration.description)
    .bind(avatar_id)
    .bind(registration.visibility.as_str())
    .bind(agent.status().as_str())
    .bind(registered_at)
    .bind(agent.version().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent.create.agent", &error))?;

    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
            principal_id, agent_id, role, granted_by, created_at
        ) VALUES (
            $1, $2, 'owner', $1,
            to_timestamp($3::double precision / 1000.0)
        )",
    )
    .bind(registration.owner_id.as_uuid())
    .bind(agent.id().as_uuid())
    .bind(registered_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent.create.ownership", &error))?;
    Ok(())
}

fn decode_agent(id: AgentId) -> impl FnOnce(PgRow) -> RepositoryResult<Agent> {
    move |row| {
        let status: String = row
            .try_get("lifecycle_state")
            .map_err(|error| map_sqlx_error("agent.decode", &error))?;
        let status =
            AgentStatus::try_from(status.as_str()).map_err(|_| corrupt_data("agent.decode"))?;
        let version = decode_version(&row, "agent.decode")?;

        Ok(Agent::restore(id, status, version))
    }
}

pub(crate) async fn find_registered_agent(
    executor: &sqlx::PgPool,
    id: AgentId,
    operation: &'static str,
) -> RepositoryResult<Option<RegisteredAgent>> {
    let row = sqlx::query(
        r"SELECT matrix_user_id, slug, display_name, description, avatar_content_id,
               visibility, lifecycle_state, version,
               floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms
          FROM agent_room.agent
          WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_optional(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| decode_registered_agent(&row, id, operation))
        .transpose()
}

pub(crate) fn decode_registered_agent(
    row: &PgRow,
    id: AgentId,
    operation: &'static str,
) -> RepositoryResult<RegisteredAgent> {
    let status: String = decode_column(row, "lifecycle_state", operation)?;
    let status = AgentStatus::try_from(status.as_str()).map_err(|_| corrupt_data(operation))?;
    let visibility: String = decode_column(row, "visibility", operation)?;
    let visibility =
        AgentVisibility::try_from(visibility.as_str()).map_err(|_| corrupt_data(operation))?;
    let avatar_content_id: Option<uuid::Uuid> = decode_column(row, "avatar_content_id", operation)?;
    Ok(RegisteredAgent {
        agent: Agent::restore(id, status, decode_version(row, operation)?),
        matrix_user_id: decode_column(row, "matrix_user_id", operation)?,
        slug: decode_column(row, "slug", operation)?,
        display_name: decode_column(row, "display_name", operation)?,
        description: decode_column(row, "description", operation)?,
        avatar_content_id: avatar_content_id.map(ContentId::from_uuid),
        visibility,
        registered_at: decode_time(row, "created_at_ms", operation)?,
    })
}

fn decode_version(row: &PgRow, operation: &'static str) -> RepositoryResult<AggregateVersion> {
    let value: i64 = row
        .try_get("version")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    AggregateVersion::new(value).map_err(|_| corrupt_data(operation))
}

pub(crate) fn decode_time(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<UtcMillis> {
    let value: i64 = decode_column(row, column, operation)?;
    UtcMillis::new(value).map_err(|_| corrupt_data(operation))
}

pub(crate) fn decode_optional_time(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<Option<UtcMillis>> {
    let value: Option<i64> = decode_column(row, column, operation)?;
    value
        .map(UtcMillis::new)
        .transpose()
        .map_err(|_| corrupt_data(operation))
}

pub(crate) fn decode_column<T>(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<T>
where
    for<'decode> T: sqlx::Decode<'decode, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

async fn classify_missing_agent(
    pool: &sqlx::PgPool,
    id: AgentId,
) -> RepositoryResult<RepositoryError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_room.agent WHERE id = $1)")
            .bind(id.as_uuid())
            .fetch_one(pool)
            .await
            .map_err(|error| map_sqlx_error("agent.save", &error))?;
    Ok(RepositoryError::new(
        "agent.save",
        if exists {
            RepositoryErrorKind::Conflict
        } else {
            RepositoryErrorKind::NotFound
        },
    ))
}

pub(crate) fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
