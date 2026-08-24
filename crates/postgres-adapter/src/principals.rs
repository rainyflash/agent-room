use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ContentPrincipalIdentityLookup, MatrixUserId, PortFuture, PrincipalRegistration,
        PrincipalRepository,
    },
};
use agent_room_domain::{
    identity::{Principal, PrincipalStatus},
    ids::PrincipalId,
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
            let avatar_id = registration
                .avatar_content_id
                .map(agent_room_domain::ids::ContentId::as_uuid);
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
        Box::pin(async move {
            let operation = "content_identity.find_active_matrix_user";
            let matrix_user_id: Option<String> = sqlx::query_scalar(
                "SELECT matrix_user_id FROM agent_room.principal WHERE id = $1 AND status = 'active'",
            )
            .bind(principal_id.as_uuid())
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
