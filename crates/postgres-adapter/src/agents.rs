use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{AgentRegistration, AgentRepository, PortFuture},
};
use agent_room_domain::{
    agents::{Agent, AgentStatus},
    ids::{AgentId, PrincipalId},
    version::AggregateVersion,
};
use sqlx::Row;

use crate::{PostgresRepositories, error::map_sqlx_error};

impl AgentRepository for PostgresRepositories {
    fn find(&self, id: AgentId) -> PortFuture<'_, RepositoryResult<Option<Agent>>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"SELECT a.lifecycle_state, a.version, ownership.principal_id
                  FROM agent_room.agent AS a
                  JOIN LATERAL (
                      SELECT principal_id
                      FROM agent_room.agent_ownership
                      WHERE agent_id = a.id
                      ORDER BY (revoked_at IS NULL) DESC, created_at, principal_id
                      LIMIT 1
                  ) AS ownership ON true
                  WHERE a.id = $1",
            )
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("agent.find", &error))?;

            row.map(decode_agent(id)).transpose()
        })
    }

    fn create<'a>(
        &'a self,
        registration: &'a AgentRegistration,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move {
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error("agent.create.begin", &error))?;
            let agent = &registration.agent;
            let avatar_id = registration
                .avatar_content_id
                .map(agent_room_domain::ids::ContentId::as_uuid);
            let registered_at = registration.registered_at.value();

            let insert_agent = sqlx::query(
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
            .execute(&mut *transaction)
            .await;
            if let Err(error) = insert_agent {
                return rollback_after_error(transaction, "agent.create", &error).await;
            }

            let insert_ownership = sqlx::query(
                r"INSERT INTO agent_room.agent_ownership (
                    principal_id, agent_id, role, granted_by, created_at
                ) VALUES (
                    $1, $2, 'owner', $1,
                    to_timestamp($3::double precision / 1000.0)
                )",
            )
            .bind(agent.owner_id().as_uuid())
            .bind(agent.id().as_uuid())
            .bind(registered_at)
            .execute(&mut *transaction)
            .await;
            if let Err(error) = insert_ownership {
                return rollback_after_error(transaction, "agent.create", &error).await;
            }

            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error("agent.create.commit", &error))?;
            Ok(agent.clone())
        })
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
                    Ok(Agent::restore(
                        agent.id(),
                        agent.owner_id(),
                        agent.status(),
                        version,
                    ))
                }
                None => Err(classify_missing_agent(&self.pool, agent.id()).await?),
            }
        })
    }
}

fn decode_agent(id: AgentId) -> impl FnOnce(sqlx::postgres::PgRow) -> RepositoryResult<Agent> {
    move |row| {
        let status: String = row
            .try_get("lifecycle_state")
            .map_err(|error| map_sqlx_error("agent.decode", &error))?;
        let status =
            AgentStatus::try_from(status.as_str()).map_err(|_| corrupt_data("agent.decode"))?;
        let owner_id: uuid::Uuid = row
            .try_get("principal_id")
            .map_err(|error| map_sqlx_error("agent.decode", &error))?;
        let version = decode_version(&row, "agent.decode")?;

        Ok(Agent::restore(
            id,
            PrincipalId::from_uuid(owner_id),
            status,
            version,
        ))
    }
}

fn decode_version(
    row: &sqlx::postgres::PgRow,
    operation: &'static str,
) -> RepositoryResult<AggregateVersion> {
    let value: i64 = row
        .try_get("version")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    AggregateVersion::new(value).map_err(|_| corrupt_data(operation))
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

async fn rollback_after_error(
    transaction: sqlx::Transaction<'_, sqlx::Postgres>,
    operation: &'static str,
    original_error: &sqlx::Error,
) -> RepositoryResult<Agent> {
    let mapped = map_sqlx_error(operation, original_error);
    transaction
        .rollback()
        .await
        .map_err(|error| map_sqlx_error("agent.create.rollback", &error))?;
    Err(mapped)
}

fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
