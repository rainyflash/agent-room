use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ContentPrincipalIdentityLookup, DirectAgentProfile, DirectSessionAgentDirectory,
        MatrixUserId, PortFuture, PrincipalRegistration, PrincipalRepository,
        PrivateRoomPrincipalDirectory,
    },
};
use agent_room_domain::{
    identity::{Principal, PrincipalStatus},
    ids::{AgentId, ContentId, PrincipalId},
    version::AggregateVersion,
};
use sqlx::Row;

use crate::{PostgresRepositories, error::map_sqlx_error};

impl PrincipalRepository for PostgresRepositories {
    fn find(&self, id: PrincipalId) -> PortFuture<'_, RepositoryResult<Option<Principal>>> {
        Box::pin(async move {
            let row = sqlx::query("SELECT status, version FROM agent_room.principal WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| map_sqlx_error("principal.find", &error))?;

            row.map(|row| decode_principal_row(&row, id, "principal.decode"))
                .transpose()
        })
    }

    fn create<'a>(
        &'a self,
        registration: &'a PrincipalRegistration,
    ) -> PortFuture<'a, RepositoryResult<Principal>> {
        Box::pin(async move {
            let principal = &registration.principal;
            let avatar_id = registration.avatar_content_id.map(ContentId::as_uuid);
            let registered_at = registration.registered_at.value();

            sqlx::query(
                r"INSERT INTO agent_room.principal (
                    id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
                    avatar_content_id, locale, status, created_at, updated_at, version
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8,
                    to_timestamp($9::double precision / 1000.0),
                    to_timestamp($9::double precision / 1000.0), $10
                )",
            )
            .bind(principal.id().as_uuid())
            .bind(&registration.oidc_issuer)
            .bind(&registration.oidc_subject)
            .bind(&registration.matrix_user_id)
            .bind(&registration.display_name)
            .bind(avatar_id)
            .bind(&registration.locale)
            .bind(principal.status().as_str())
            .bind(registered_at)
            .bind(principal.version().value())
            .execute(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("principal.create", &error))?;

            Ok(principal.clone())
        })
    }

    fn save<'a>(&'a self, principal: &'a Principal) -> PortFuture<'a, RepositoryResult<Principal>> {
        Box::pin(async move {
            let next_version = principal
                .version()
                .next()
                .map_err(|_| corrupt_data("principal.save"))?;
            let row = sqlx::query(
                r"UPDATE agent_room.principal
                   SET status = $2, updated_at = clock_timestamp(), version = $3
                   WHERE id = $1 AND version = $4
                   RETURNING version",
            )
            .bind(principal.id().as_uuid())
            .bind(principal.status().as_str())
            .bind(next_version.value())
            .bind(principal.version().value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("principal.save", &error))?;

            match row {
                Some(row) => {
                    let version = decode_version(&row, "principal.save")?;
                    Ok(Principal::restore(
                        principal.id(),
                        principal.status(),
                        version,
                    ))
                }
                None => Err(classify_missing_principal(&self.pool, principal.id()).await?),
            }
        })
    }
}

impl ContentPrincipalIdentityLookup for PostgresRepositories {
    fn find_active_matrix_user(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(find_active_principal_matrix_user(
            &self.pool,
            principal_id,
            "content_identity.find_active_matrix_user",
        ))
    }

    fn find_active_agent_matrix_user(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(async move {
            let operation = "content_identity.find_active_agent_matrix_user";
            let matrix_user_id: Option<String> = sqlx::query_scalar(
                r"SELECT agent.matrix_user_id
                  FROM agent_room.principal AS principal
                  JOIN agent_room.agent_ownership AS ownership
                    ON ownership.principal_id = principal.id
                   AND ownership.agent_id = $2
                   AND ownership.revoked_at IS NULL
                   AND ownership.role IN ('owner', 'operator')
                  JOIN agent_room.agent AS agent
                    ON agent.id = ownership.agent_id
                   AND agent.lifecycle_state = 'active'
                  WHERE principal.id = $1 AND principal.status = 'active'",
            )
            .bind(principal_id.as_uuid())
            .bind(agent_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            matrix_user_id
                .map(MatrixUserId::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))
        })
    }
}

impl PrivateRoomPrincipalDirectory for PostgresRepositories {
    fn matrix_user_id(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(find_active_principal_matrix_user(
            &self.pool,
            principal_id,
            "private_room_principal.matrix_user_id",
        ))
    }
}

impl DirectSessionAgentDirectory for PostgresRepositories {
    fn find_contactable(
        &self,
        actor_principal_id: PrincipalId,
        target_agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectAgentProfile>>> {
        Box::pin(find_direct_agent_profile(
            &self.pool,
            actor_principal_id,
            target_agent_id,
            false,
            "direct_session_agent.find_contactable",
        ))
    }

    fn find_known_contact(
        &self,
        actor_principal_id: PrincipalId,
        target_agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<DirectAgentProfile>>> {
        Box::pin(find_direct_agent_profile(
            &self.pool,
            actor_principal_id,
            target_agent_id,
            true,
            "direct_session_agent.find_known_contact",
        ))
    }
}

async fn find_direct_agent_profile(
    pool: &sqlx::PgPool,
    actor_principal_id: PrincipalId,
    target_agent_id: AgentId,
    include_known_relationship: bool,
    operation: &'static str,
) -> RepositoryResult<Option<DirectAgentProfile>> {
    let row = sqlx::query(
        r"SELECT agent.matrix_user_id, agent.display_name, agent.avatar_content_id
           FROM agent_room.agent AS agent
           WHERE agent.id = $2
             AND (
                 (
                     agent.lifecycle_state = 'active'
                     AND (
                         agent.visibility <> 'private'
                         OR EXISTS (
                             SELECT 1
                             FROM agent_room.agent_ownership AS ownership
                             WHERE ownership.principal_id = $1
                               AND ownership.agent_id = agent.id
                               AND ownership.revoked_at IS NULL
                         )
                     )
                 )
                 OR (
                     $3::boolean
                     AND (
                         EXISTS (
                             SELECT 1
                             FROM agent_room.direct_session AS direct_session
                             WHERE direct_session.principal_id = $1
                               AND direct_session.target_agent_id = agent.id
                         )
                         OR EXISTS (
                             SELECT 1
                             FROM agent_room.direct_contact_block AS contact_block
                             WHERE contact_block.principal_id = $1
                               AND contact_block.agent_id = agent.id
                               AND contact_block.revoked_at IS NULL
                         )
                         OR EXISTS (
                             SELECT 1
                             FROM agent_room.agent_ownership AS ownership
                             WHERE ownership.principal_id = $1
                               AND ownership.agent_id = agent.id
                               AND ownership.revoked_at IS NULL
                         )
                     )
                 )
             )",
    )
    .bind(actor_principal_id.as_uuid())
    .bind(target_agent_id.as_uuid())
    .bind(include_known_relationship)
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| decode_direct_agent_profile(&row, target_agent_id, operation))
        .transpose()
}

fn decode_direct_agent_profile(
    row: &sqlx::postgres::PgRow,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<DirectAgentProfile> {
    let matrix_user_id: String = row
        .try_get("matrix_user_id")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let avatar_content_id: Option<uuid::Uuid> = row
        .try_get("avatar_content_id")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(DirectAgentProfile {
        agent_id,
        matrix_user_id: MatrixUserId::new(matrix_user_id).map_err(|_| corrupt_data(operation))?,
        display_name: row
            .try_get("display_name")
            .map_err(|error| map_sqlx_error(operation, &error))?,
        avatar_content_id: avatar_content_id.map(ContentId::from_uuid),
    })
}

async fn find_active_principal_matrix_user(
    pool: &sqlx::PgPool,
    principal_id: PrincipalId,
    operation: &'static str,
) -> RepositoryResult<Option<MatrixUserId>> {
    let matrix_user_id: Option<String> = sqlx::query_scalar(
        "SELECT matrix_user_id FROM agent_room.principal WHERE id = $1 AND status = 'active'",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    matrix_user_id
        .map(MatrixUserId::new)
        .transpose()
        .map_err(|_| corrupt_data(operation))
}

pub(crate) fn decode_principal_row(
    row: &sqlx::postgres::PgRow,
    id: PrincipalId,
    operation: &'static str,
) -> RepositoryResult<Principal> {
    let status: String = row
        .try_get("status")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let status = PrincipalStatus::try_from(status.as_str()).map_err(|_| corrupt_data(operation))?;
    let version = decode_version(row, operation)?;
    Ok(Principal::restore(id, status, version))
}

pub(crate) fn decode_version(
    row: &sqlx::postgres::PgRow,
    operation: &'static str,
) -> RepositoryResult<AggregateVersion> {
    let value: i64 = row
        .try_get("version")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    AggregateVersion::new(value).map_err(|_| corrupt_data(operation))
}

async fn classify_missing_principal(
    pool: &sqlx::PgPool,
    id: PrincipalId,
) -> RepositoryResult<RepositoryError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_room.principal WHERE id = $1)")
            .bind(id.as_uuid())
            .fetch_one(pool)
            .await
            .map_err(|error| map_sqlx_error("principal.save", &error))?;
    Ok(RepositoryError::new(
        "principal.save",
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
