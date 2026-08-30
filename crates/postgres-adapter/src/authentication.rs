use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        DesktopAuthorizationCodeRegistration, DesktopClientState,
        DesktopLoginCompletionTransaction, DesktopSessionExchangeTransaction,
        DesktopSessionRegistration, LoginAttempt, LoginAttemptStore, LoginCompletionTransaction,
        LoginDelivery, PkceCodeChallenge, PortFuture, PrincipalAccount, PrincipalRegistration,
        PrincipalSuspensionTransaction, SafeReturnPath, SecretDigest, SecretValue,
        StoredWebSession, WebSessionRegistration, WebSessionStore,
    },
};
use agent_room_domain::{
    identity::PrincipalStatus,
    ids::{ContentId, LoginAttemptId, PrincipalId, WebSessionId},
    time::UtcMillis,
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    error::map_sqlx_error,
    principals::{corrupt_data, decode_principal_row},
};

impl LoginAttemptStore for PostgresRepositories {
    fn create<'a>(&'a self, attempt: &'a LoginAttempt) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let (delivery_kind, desktop_client_state, desktop_pkce_challenge) =
                match &attempt.delivery {
                    LoginDelivery::Web { .. } => ("web", None, None),
                    LoginDelivery::Desktop {
                        client_state,
                        code_challenge,
                        ..
                    } => (
                        "desktop",
                        Some(client_state.expose()),
                        Some(code_challenge.as_str()),
                    ),
                };
            sqlx::query(
                r"INSERT INTO agent_room.oidc_login_attempt (
                    id, browser_secret_digest, state_digest, nonce, pkce_verifier,
                    return_path, import_display_name, import_locale, created_at, expires_at,
                    delivery_kind, desktop_client_state, desktop_pkce_challenge
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8,
                    to_timestamp($9::double precision / 1000.0),
                    to_timestamp($10::double precision / 1000.0), $11, $12, $13
                )",
            )
            .bind(attempt.id.as_uuid())
            .bind(attempt.browser_secret_digest.as_bytes().as_slice())
            .bind(attempt.state_digest.as_bytes().as_slice())
            .bind(attempt.nonce.expose())
            .bind(attempt.pkce_verifier.expose())
            .bind(attempt.delivery.return_path().as_str())
            .bind(attempt.profile_import.display_name)
            .bind(attempt.profile_import.locale)
            .bind(attempt.created_at.value())
            .bind(attempt.expires_at.value())
            .bind(delivery_kind)
            .bind(desktop_client_state)
            .bind(desktop_pkce_challenge)
            .execute(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("login_attempt.create", &error))?;
            Ok(())
        })
    }

    fn consume<'a>(
        &'a self,
        browser_secret_digest: &'a SecretDigest,
        state_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<LoginAttempt>>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"UPDATE agent_room.oidc_login_attempt
                   SET consumed_at = to_timestamp($3::double precision / 1000.0)
                   WHERE browser_secret_digest = $1
                     AND state_digest = $2
                     AND consumed_at IS NULL
                     AND expires_at > to_timestamp($3::double precision / 1000.0)
                   RETURNING id, nonce, pkce_verifier, return_path, delivery_kind,
                     desktop_client_state, desktop_pkce_challenge,
                     import_display_name, import_locale,
                     floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms,
                     floor(extract(epoch FROM expires_at) * 1000)::bigint AS expires_at_ms",
            )
            .bind(browser_secret_digest.as_bytes().as_slice())
            .bind(state_digest.as_bytes().as_slice())
            .bind(now.value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?;

            row.map(|row| decode_login_attempt(&row, *browser_secret_digest, *state_digest))
                .transpose()
        })
    }
}

impl LoginCompletionTransaction for PostgresRepositories {
    fn complete<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        session: &'a WebSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredWebSession>> {
        Box::pin(async move {
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error("login.complete", &error))?;
            insert_principal_if_absent(&mut transaction, principal, "login.complete").await?;
            let account = lock_account_by_oidc(
                &mut transaction,
                &principal.oidc_issuer,
                &principal.oidc_subject,
                "login.complete",
            )
            .await?;
            if !account.principal.allows_authentication() {
                transaction
                    .rollback()
                    .await
                    .map_err(|error| map_sqlx_error("login.complete", &error))?;
                return Err(RepositoryError::new(
                    "login.complete",
                    RepositoryErrorKind::Forbidden,
                ));
            }
            insert_web_session(
                &mut transaction,
                account.principal.id(),
                session,
                "login.complete",
            )
            .await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error("login.complete", &error))?;

            Ok(StoredWebSession {
                id: session.id,
                account,
                authenticated_at: session.authenticated_at,
                created_at: session.created_at,
                expires_at: session.expires_at,
            })
        })
    }
}

impl DesktopLoginCompletionTransaction for PostgresRepositories {
    fn complete_desktop<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        authorization: &'a DesktopAuthorizationCodeRegistration,
    ) -> PortFuture<'a, RepositoryResult<PrincipalAccount>> {
        Box::pin(async move {
            let operation = "desktop_login.complete";
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            insert_principal_if_absent(&mut transaction, principal, operation).await?;
            let account = lock_account_by_oidc(
                &mut transaction,
                &principal.oidc_issuer,
                &principal.oidc_subject,
                operation,
            )
            .await?;
            if !account.principal.allows_authentication() {
                transaction
                    .rollback()
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Forbidden,
                ));
            }
            sqlx::query(
                r"INSERT INTO agent_room.desktop_authorization_code (
                    code_digest, principal_id, pkce_challenge, authenticated_at,
                    created_at, expires_at
                ) VALUES (
                    $1, $2, $3,
                    to_timestamp($4::double precision / 1000.0),
                    to_timestamp($5::double precision / 1000.0),
                    to_timestamp($6::double precision / 1000.0)
                )",
            )
            .bind(authorization.code_digest.as_bytes().as_slice())
            .bind(account.principal.id().as_uuid())
            .bind(authorization.code_challenge.as_str())
            .bind(authorization.authenticated_at.value())
            .bind(authorization.created_at.value())
            .bind(authorization.expires_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(account)
        })
    }
}

impl DesktopSessionExchangeTransaction for PostgresRepositories {
    fn exchange_desktop<'a>(
        &'a self,
        code_digest: &'a SecretDigest,
        code_challenge: &'a PkceCodeChallenge,
        session: &'a DesktopSessionRegistration,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>> {
        Box::pin(async move {
            let operation = "desktop_session.exchange";
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let authorization = sqlx::query(
                r"UPDATE agent_room.desktop_authorization_code
                   SET consumed_at = to_timestamp($3::double precision / 1000.0)
                   WHERE code_digest = $1
                     AND pkce_challenge = $2
                     AND consumed_at IS NULL
                     AND expires_at > to_timestamp($3::double precision / 1000.0)
                   RETURNING principal_id,
                     floor(extract(epoch FROM authenticated_at) * 1000)::bigint
                       AS authenticated_at_ms",
            )
            .bind(code_digest.as_bytes().as_slice())
            .bind(code_challenge.as_str())
            .bind(now.value())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let Some(authorization) = authorization else {
                transaction
                    .rollback()
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
                return Ok(None);
            };
            let principal_id =
                PrincipalId::from_uuid(decode_uuid(&authorization, "principal_id", operation)?);
            let authenticated_at = decode_time(&authorization, "authenticated_at_ms", operation)?;
            let account = lock_account_by_id(&mut transaction, principal_id, operation).await?;
            if !account.principal.allows_authentication() {
                transaction
                    .rollback()
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Forbidden,
                ));
            }
            let web_session = WebSessionRegistration {
                id: session.id,
                secret_digest: session.secret_digest,
                authenticated_at,
                created_at: session.created_at,
                expires_at: session.expires_at,
            };
            insert_web_session(&mut transaction, principal_id, &web_session, operation).await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(Some(StoredWebSession {
                id: session.id,
                account,
                authenticated_at,
                created_at: session.created_at,
                expires_at: session.expires_at,
            }))
        })
    }
}

impl WebSessionStore for PostgresRepositories {
    fn find_active<'a>(
        &'a self,
        secret_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>> {
        Box::pin(async move {
            let row = sqlx::query(
                r"SELECT session.id AS session_id, principal.id AS principal_id,
                     principal.status, principal.version, principal.matrix_user_id,
                     principal.display_name, principal.avatar_content_id, principal.locale,
                     floor(extract(epoch FROM session.authenticated_at) * 1000)::bigint
                       AS authenticated_at_ms,
                     floor(extract(epoch FROM session.created_at) * 1000)::bigint
                       AS created_at_ms,
                     floor(extract(epoch FROM session.expires_at) * 1000)::bigint
                       AS expires_at_ms
                   FROM agent_room.web_session AS session
                   JOIN agent_room.principal AS principal ON principal.id = session.principal_id
                   WHERE session.secret_digest = $1
                     AND session.revoked_at IS NULL
                     AND session.expires_at > to_timestamp($2::double precision / 1000.0)",
            )
            .bind(secret_digest.as_bytes().as_slice())
            .bind(now.value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("web_session.find_active", &error))?;

            row.map(|row| decode_stored_session(&row)).transpose()
        })
    }

    fn revoke<'a>(
        &'a self,
        secret_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let result = sqlx::query(
                r"UPDATE agent_room.web_session
                   SET revoked_at = to_timestamp($2::double precision / 1000.0)
                   WHERE secret_digest = $1 AND revoked_at IS NULL",
            )
            .bind(secret_digest.as_bytes().as_slice())
            .bind(now.value())
            .execute(&self.pool)
            .await
            .map_err(|error| map_sqlx_error("web_session.revoke", &error))?;
            Ok(result.rows_affected() == 1)
        })
    }
}

impl PrincipalSuspensionTransaction for PostgresRepositories {
    fn suspend(
        &self,
        principal_id: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<agent_room_domain::identity::Principal>> {
        Box::pin(async move {
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error("principal.suspend", &error))?;
            let row = sqlx::query(
                "SELECT status, version FROM agent_room.principal WHERE id = $1 FOR UPDATE",
            )
            .bind(principal_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error("principal.suspend", &error))?
            .ok_or_else(|| {
                RepositoryError::new("principal.suspend", RepositoryErrorKind::NotFound)
            })?;
            let mut principal = decode_principal_row(&row, principal_id, "principal.suspend")?;
            if !matches!(
                principal.status(),
                PrincipalStatus::Active | PrincipalStatus::Suspended
            ) {
                transaction
                    .rollback()
                    .await
                    .map_err(|error| map_sqlx_error("principal.suspend", &error))?;
                return Err(RepositoryError::new(
                    "principal.suspend",
                    RepositoryErrorKind::Forbidden,
                ));
            }

            if principal.status() == PrincipalStatus::Active {
                principal
                    .suspend()
                    .map_err(|_| corrupt_data("principal.suspend"))?;
                let next_version = principal
                    .version()
                    .next()
                    .map_err(|_| corrupt_data("principal.suspend"))?;
                sqlx::query(
                    r"UPDATE agent_room.principal
                       SET status = 'suspended', version = $2,
                           updated_at = to_timestamp($3::double precision / 1000.0)
                       WHERE id = $1",
                )
                .bind(principal_id.as_uuid())
                .bind(next_version.value())
                .bind(now.value())
                .execute(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error("principal.suspend", &error))?;
                principal.restore_version(next_version);
            }
            sqlx::query(
                r"UPDATE agent_room.web_session
                   SET revoked_at = to_timestamp($2::double precision / 1000.0)
                   WHERE principal_id = $1 AND revoked_at IS NULL",
            )
            .bind(principal_id.as_uuid())
            .bind(now.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error("principal.suspend", &error))?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error("principal.suspend", &error))?;
            Ok(principal)
        })
    }
}

pub(crate) async fn insert_principal_if_absent(
    transaction: &mut Transaction<'_, Postgres>,
    registration: &PrincipalRegistration,
    operation: &'static str,
) -> RepositoryResult<()> {
    let avatar_id = registration.avatar_content_id.map(ContentId::as_uuid);
    sqlx::query(
        r"INSERT INTO agent_room.principal (
            id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
            avatar_content_id, locale, status, created_at, updated_at, version
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            to_timestamp($9::double precision / 1000.0),
            to_timestamp($9::double precision / 1000.0), $10
        ) ON CONFLICT (oidc_issuer, oidc_subject) DO NOTHING",
    )
    .bind(registration.principal.id().as_uuid())
    .bind(&registration.oidc_issuer)
    .bind(&registration.oidc_subject)
    .bind(&registration.matrix_user_id)
    .bind(&registration.display_name)
    .bind(avatar_id)
    .bind(&registration.locale)
    .bind(registration.principal.status().as_str())
    .bind(registration.registered_at.value())
    .bind(registration.principal.version().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

pub(crate) async fn lock_account_by_oidc(
    transaction: &mut Transaction<'_, Postgres>,
    issuer: &str,
    subject: &str,
    operation: &'static str,
) -> RepositoryResult<PrincipalAccount> {
    let row = sqlx::query(
        r"SELECT id AS principal_id, status, version, matrix_user_id, display_name,
             avatar_content_id, locale
           FROM agent_room.principal
           WHERE oidc_issuer = $1 AND oidc_subject = $2
           FOR UPDATE",
    )
    .bind(issuer)
    .bind(subject)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))?;
    decode_principal_account(&row, operation)
}

async fn lock_account_by_id(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    operation: &'static str,
) -> RepositoryResult<PrincipalAccount> {
    let row = sqlx::query(
        r"SELECT id AS principal_id, status, version, matrix_user_id, display_name,
             avatar_content_id, locale
           FROM agent_room.principal
           WHERE id = $1
           FOR UPDATE",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))?;
    decode_principal_account(&row, operation)
}

async fn insert_web_session(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    session: &WebSessionRegistration,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.web_session (
            id, principal_id, secret_digest, authenticated_at, created_at, expires_at
        ) VALUES (
            $1, $2, $3,
            to_timestamp($4::double precision / 1000.0),
            to_timestamp($5::double precision / 1000.0),
            to_timestamp($6::double precision / 1000.0)
        )",
    )
    .bind(session.id.as_uuid())
    .bind(principal_id.as_uuid())
    .bind(session.secret_digest.as_bytes().as_slice())
    .bind(session.authenticated_at.value())
    .bind(session.created_at.value())
    .bind(session.expires_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_login_attempt(
    row: &PgRow,
    browser_secret_digest: SecretDigest,
    state_digest: SecretDigest,
) -> RepositoryResult<LoginAttempt> {
    let id = LoginAttemptId::from_uuid(decode_uuid(row, "id", "login_attempt.consume")?);
    let nonce = decode_secret(row, "nonce", "login_attempt.consume")?;
    let pkce_verifier = decode_secret(row, "pkce_verifier", "login_attempt.consume")?;
    let return_path: String = row
        .try_get("return_path")
        .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?;
    let return_path =
        SafeReturnPath::new(return_path).map_err(|_| corrupt_data("login_attempt.consume"))?;
    let delivery_kind: String = row
        .try_get("delivery_kind")
        .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?;
    let delivery = match delivery_kind.as_str() {
        "web" => LoginDelivery::Web { return_path },
        "desktop" => {
            let client_state: String = row
                .try_get("desktop_client_state")
                .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?;
            let code_challenge: String = row
                .try_get("desktop_pkce_challenge")
                .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?;
            LoginDelivery::Desktop {
                client_state: DesktopClientState::new(client_state)
                    .map_err(|_| corrupt_data("login_attempt.consume"))?,
                code_challenge: PkceCodeChallenge::new(code_challenge)
                    .map_err(|_| corrupt_data("login_attempt.consume"))?,
                return_path,
            }
        }
        _ => return Err(corrupt_data("login_attempt.consume")),
    };
    Ok(LoginAttempt {
        id,
        browser_secret_digest,
        state_digest,
        nonce,
        pkce_verifier,
        delivery,
        profile_import: agent_room_application::ports::ProfileImportConsent {
            display_name: row
                .try_get("import_display_name")
                .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?,
            locale: row
                .try_get("import_locale")
                .map_err(|error| map_sqlx_error("login_attempt.consume", &error))?,
        },
        created_at: decode_time(row, "created_at_ms", "login_attempt.consume")?,
        expires_at: decode_time(row, "expires_at_ms", "login_attempt.consume")?,
    })
}

fn decode_stored_session(row: &PgRow) -> RepositoryResult<StoredWebSession> {
    Ok(StoredWebSession {
        id: WebSessionId::from_uuid(decode_uuid(row, "session_id", "web_session.find_active")?),
        account: decode_principal_account(row, "web_session.find_active")?,
        authenticated_at: decode_time(row, "authenticated_at_ms", "web_session.find_active")?,
        created_at: decode_time(row, "created_at_ms", "web_session.find_active")?,
        expires_at: decode_time(row, "expires_at_ms", "web_session.find_active")?,
    })
}

pub(crate) fn decode_principal_account(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<PrincipalAccount> {
    let id = PrincipalId::from_uuid(decode_uuid(row, "principal_id", operation)?);
    let principal = decode_principal_row(row, id, operation)?;
    let avatar_content_id = row
        .try_get::<Option<Uuid>, _>("avatar_content_id")
        .map_err(|error| map_sqlx_error(operation, &error))?
        .map(ContentId::from_uuid);
    Ok(PrincipalAccount {
        principal,
        matrix_user_id: row
            .try_get("matrix_user_id")
            .map_err(|error| map_sqlx_error(operation, &error))?,
        display_name: row
            .try_get("display_name")
            .map_err(|error| map_sqlx_error(operation, &error))?,
        avatar_content_id,
        locale: row
            .try_get("locale")
            .map_err(|error| map_sqlx_error(operation, &error))?,
    })
}

fn decode_secret(
    row: &PgRow,
    column: &'static str,
    operation: &'static str,
) -> RepositoryResult<SecretValue> {
    let value: String = row
        .try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))?;
    SecretValue::new(value).map_err(|_| corrupt_data(operation))
}

fn decode_uuid(
    row: &PgRow,
    column: &'static str,
    operation: &'static str,
) -> RepositoryResult<Uuid> {
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

fn decode_time(
    row: &PgRow,
    column: &'static str,
    operation: &'static str,
) -> RepositoryResult<UtcMillis> {
    let value: i64 = row
        .try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))?;
    UtcMillis::new(value).map_err(|_| corrupt_data(operation))
}
