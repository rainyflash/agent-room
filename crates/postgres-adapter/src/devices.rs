use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        DeviceProofNonceStore, DeviceRefreshContext, DeviceRefreshOutcome,
        DeviceRegistrationTransaction, DeviceRepository, DeviceRevocationOutcome,
        DeviceRevocationTransaction, DeviceSecurityEvent, DeviceSessionRegistration,
        DeviceSessionStore, DeviceTokenReplacement, OutboxMessage,
        PendingAgentMatrixDeviceRevocation, PortFuture, PrincipalAccount, PrincipalRegistration,
        SecretDigest, StoredDeviceSession,
    },
};
use agent_room_domain::{
    devices::{
        Device, DevicePlatform, DevicePublicSigningKey, DeviceTokenFamily, DeviceTokenFamilyState,
        DeviceTrustState,
    },
    ids::{AgentInstanceId, DeviceId, DeviceTokenFamilyId, PrincipalId},
    time::UtcMillis,
};
use serde_json::{Map, Value};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    authentication::{decode_principal_account, insert_principal_if_absent, lock_account_by_oidc},
    error::map_sqlx_error,
    handoffs::fail_targeted_handoffs_for_device,
    outbox::insert_outbox_event,
};

const DEVICE_SELECT: &str = r"device.id AS device_id,
    device.principal_id AS device_principal_id, device.label AS device_label,
    device.platform AS device_platform, device.public_signing_key,
    device.matrix_device_id, device.trust_state AS device_trust_state,
    floor(extract(epoch FROM device.last_seen_at) * 1000)::bigint AS device_last_seen_at_ms,
    floor(extract(epoch FROM device.revoked_at) * 1000)::bigint AS device_revoked_at_ms,
    floor(extract(epoch FROM device.created_at) * 1000)::bigint AS device_created_at_ms";

const PRINCIPAL_SELECT: &str = r"principal.id AS principal_id, principal.status,
    principal.version, principal.matrix_user_id, principal.display_name,
    principal.avatar_content_id, principal.locale";

const FAMILY_SELECT: &str = r"family.id AS family_id, family.device_id AS family_device_id,
    family.state AS family_state,
    floor(extract(epoch FROM family.created_at) * 1000)::bigint AS family_created_at_ms,
    floor(extract(epoch FROM family.expires_at) * 1000)::bigint AS family_expires_at_ms,
    floor(extract(epoch FROM family.revoked_at) * 1000)::bigint AS family_revoked_at_ms,
    floor(extract(epoch FROM family.compromise_detected_at) * 1000)::bigint
        AS family_compromise_detected_at_ms";

impl DeviceRegistrationTransaction for PostgresRepositories {
    fn register<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        requested_device: &'a Device,
        session: &'a DeviceSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredDeviceSession>> {
        Box::pin(register_device_transaction(
            &self.pool,
            principal,
            requested_device,
            session,
        ))
    }
}

impl DeviceSessionStore for PostgresRepositories {
    fn find_active_access<'a>(
        &'a self,
        access_token_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredDeviceSession>>> {
        Box::pin(async move {
            let operation = "device_access.find_active";
            let query = format!(
                r"SELECT {PRINCIPAL_SELECT}, {DEVICE_SELECT}, {FAMILY_SELECT},
                    floor(extract(epoch FROM access.expires_at) * 1000)::bigint
                        AS access_expires_at_ms
                   FROM agent_room.device_access_token AS access
                   JOIN agent_room.device_token_family AS family ON family.id = access.family_id
                   JOIN agent_room.device AS device ON device.id = access.device_id
                   JOIN agent_room.principal AS principal ON principal.id = device.principal_id
                   WHERE access.secret_digest = $1
                     AND access.revoked_at IS NULL
                     AND access.expires_at > to_timestamp($2::double precision / 1000.0)
                     AND family.state = 'active'
                     AND family.expires_at > to_timestamp($2::double precision / 1000.0)
                     AND device.trust_state = 'verified'
                     AND principal.status = 'active'"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(access_token_digest.as_bytes().as_slice())
                .bind(now.value())
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_stored_device_session(&row, operation))
                .transpose()
        })
    }

    fn find_refresh_context<'a>(
        &'a self,
        refresh_token_digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<DeviceRefreshContext>>> {
        Box::pin(async move {
            let operation = "device_refresh.find_context";
            let query = format!(
                r"SELECT {PRINCIPAL_SELECT}, {DEVICE_SELECT}, {FAMILY_SELECT}
                   FROM agent_room.device_refresh_token AS refresh
                   JOIN agent_room.device_token_family AS family ON family.id = refresh.family_id
                   JOIN agent_room.device AS device ON device.id = family.device_id
                   JOIN agent_room.principal AS principal ON principal.id = device.principal_id
                   WHERE refresh.secret_digest = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(refresh_token_digest.as_bytes().as_slice())
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| {
                Ok(DeviceRefreshContext {
                    account: decode_principal_account(&row, operation)?,
                    device: decode_device(&row, operation)?,
                    family: decode_token_family(&row, operation)?,
                })
            })
            .transpose()
        })
    }

    fn rotate_refresh<'a>(
        &'a self,
        refresh_token_digest: &'a SecretDigest,
        replacement: &'a DeviceTokenReplacement,
        security_event: DeviceSecurityEvent,
    ) -> PortFuture<'a, RepositoryResult<DeviceRefreshOutcome>> {
        Box::pin(async move {
            rotate_refresh_transaction(
                &self.pool,
                refresh_token_digest,
                replacement,
                security_event,
            )
            .await
        })
    }
}

impl DeviceProofNonceStore for PostgresRepositories {
    fn consume<'a>(
        &'a self,
        device_id: DeviceId,
        nonce_digest: &'a SecretDigest,
        consumed_at: UtcMillis,
        expires_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "device_nonce.consume";
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            sqlx::query(
                r"DELETE FROM agent_room.device_proof_nonce
                   WHERE device_id = $1
                     AND expires_at < to_timestamp($2::double precision / 1000.0)",
            )
            .bind(device_id.as_uuid())
            .bind(consumed_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let inserted = sqlx::query(
                r"INSERT INTO agent_room.device_proof_nonce (
                    device_id, nonce_digest, consumed_at, expires_at
                ) VALUES (
                    $1, $2,
                    to_timestamp($3::double precision / 1000.0),
                    to_timestamp($4::double precision / 1000.0)
                ) ON CONFLICT (device_id, nonce_digest) DO NOTHING",
            )
            .bind(device_id.as_uuid())
            .bind(nonce_digest.as_bytes().as_slice())
            .bind(consumed_at.value())
            .bind(expires_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .rows_affected()
                == 1;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(inserted)
        })
    }
}

impl DeviceRepository for PostgresRepositories {
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<Device>>> {
        Box::pin(async move {
            let operation = "device.list";
            let query = format!(
                r"SELECT {DEVICE_SELECT}
                   FROM agent_room.device AS device
                   WHERE device.principal_id = $1
                   ORDER BY device.created_at DESC, device.id DESC"
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(principal_id.as_uuid())
                .fetch_all(&self.pool)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_device(row, operation))
                .collect()
        })
    }
}

impl DeviceRevocationTransaction for PostgresRepositories {
    fn revoke(
        &self,
        principal_id: PrincipalId,
        device_id: DeviceId,
        security_event: DeviceSecurityEvent,
    ) -> PortFuture<'_, RepositoryResult<DeviceRevocationOutcome>> {
        Box::pin(async move {
            revoke_device_transaction(&self.pool, principal_id, device_id, security_event).await
        })
    }
}

async fn register_device_transaction(
    pool: &sqlx::PgPool,
    principal: &PrincipalRegistration,
    requested_device: &Device,
    session: &DeviceSessionRegistration,
) -> RepositoryResult<StoredDeviceSession> {
    let operation = "device.register";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let (account, device) =
        resolve_device_owner(&mut transaction, principal, requested_device, operation).await?;
    consume_authorization_receipt(
        &mut transaction,
        account.principal.id(),
        device.id(),
        session,
        operation,
    )
    .await?;
    revoke_active_families(&mut transaction, device.id(), session.issued_at, operation).await?;
    let family = DeviceTokenFamily::new(
        session.family.id(),
        device.id(),
        session.family.created_at(),
        session.family.expires_at(),
    )
    .map_err(|_| corrupt_data(operation))?;
    insert_token_family(&mut transaction, &family, operation).await?;
    insert_access_token(
        &mut transaction,
        device.id(),
        family.id(),
        session.access_token_id.as_uuid(),
        &session.access_token_digest,
        session.issued_at,
        session.access_token_expires_at,
        operation,
    )
    .await?;
    insert_refresh_token(
        &mut transaction,
        family.id(),
        session.refresh_token_id.as_uuid(),
        &session.refresh_token_digest,
        0,
        session.issued_at,
        family.expires_at(),
        operation,
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(StoredDeviceSession {
        account,
        device,
        family,
        access_token_expires_at: session.access_token_expires_at,
    })
}

async fn consume_authorization_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    device_id: DeviceId,
    session: &DeviceSessionRegistration,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"DELETE FROM agent_room.device_authorization_receipt
           WHERE expires_at < to_timestamp($1::double precision / 1000.0)",
    )
    .bind(session.issued_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    sqlx::query(
        r"INSERT INTO agent_room.device_authorization_receipt (
            authorization_digest, principal_id, device_id, consumed_at, expires_at
        ) VALUES (
            $1, $2, $3,
            to_timestamp($4::double precision / 1000.0),
            to_timestamp($5::double precision / 1000.0)
        )",
    )
    .bind(session.authorization_token_digest.as_bytes().as_slice())
    .bind(principal_id.as_uuid())
    .bind(device_id.as_uuid())
    .bind(session.issued_at.value())
    .bind(session.authorization_receipt_expires_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn resolve_device_owner(
    transaction: &mut Transaction<'_, Postgres>,
    principal: &PrincipalRegistration,
    requested_device: &Device,
    operation: &'static str,
) -> RepositoryResult<(PrincipalAccount, Device)> {
    insert_principal_if_absent(transaction, principal, operation).await?;
    let account = lock_account_by_oidc(
        transaction,
        &principal.oidc_issuer,
        &principal.oidc_subject,
        operation,
    )
    .await?;
    if !account.principal.allows_authentication() {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ));
    }
    insert_device_if_absent(
        transaction,
        account.principal.id(),
        requested_device,
        operation,
    )
    .await?;
    let existing = lock_device_by_active_key(
        transaction,
        requested_device.public_signing_key(),
        operation,
    )
    .await?;
    if existing.principal_id() != account.principal.id() {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ));
    }
    update_verified_device(transaction, &existing, requested_device, operation).await?;
    let device = Device::restore(
        existing.id(),
        existing.principal_id(),
        requested_device.label().to_owned(),
        requested_device.platform(),
        requested_device.public_signing_key().clone(),
        existing.matrix_device_id().map(str::to_owned),
        DeviceTrustState::Verified,
        existing.last_seen_at(),
        None,
        existing.created_at(),
    )
    .map_err(|_| corrupt_data(operation))?;
    Ok((account, device))
}

async fn rotate_refresh_transaction(
    pool: &sqlx::PgPool,
    refresh_token_digest: &SecretDigest,
    replacement: &DeviceTokenReplacement,
    security_event: DeviceSecurityEvent,
) -> RepositoryResult<DeviceRefreshOutcome> {
    let operation = "device.refresh";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let Some(locked) =
        lock_refresh_context(&mut transaction, refresh_token_digest, operation).await?
    else {
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        return Ok(DeviceRefreshOutcome::Rejected);
    };
    if !locked.account.principal.allows_authentication()
        || !locked.device.accepts_authenticated_requests()
        || !locked.family.allows_rotation(replacement.issued_at)
        || locked.revoked
        || replacement.issued_at >= locked.expires_at
    {
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        return Ok(DeviceRefreshOutcome::Rejected);
    }
    if locked.consumed {
        compromise_device(
            &mut transaction,
            &locked.device,
            locked.family.id(),
            security_event,
            operation,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        return Ok(DeviceRefreshOutcome::ReuseDetected {
            device_id: locked.device.id(),
            principal_id: locked.account.principal.id(),
        });
    }

    let session =
        persist_refresh_replacement(&mut transaction, &locked, replacement, operation).await?;
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;

    Ok(DeviceRefreshOutcome::Rotated {
        refresh_token_expires_at: session.family.expires_at(),
        session: Box::new(session),
    })
}

struct LockedRefreshContext {
    account: PrincipalAccount,
    device: Device,
    family: DeviceTokenFamily,
    refresh_id: Uuid,
    sequence: i64,
    consumed: bool,
    revoked: bool,
    expires_at: UtcMillis,
}

async fn lock_refresh_context(
    transaction: &mut Transaction<'_, Postgres>,
    refresh_token_digest: &SecretDigest,
    operation: &'static str,
) -> RepositoryResult<Option<LockedRefreshContext>> {
    let query = format!(
        r"SELECT {PRINCIPAL_SELECT}, {DEVICE_SELECT}, {FAMILY_SELECT},
            refresh.id AS refresh_id, refresh.sequence,
            refresh.consumed_at IS NOT NULL AS refresh_consumed,
            refresh.revoked_at IS NOT NULL AS refresh_revoked,
            floor(extract(epoch FROM refresh.expires_at) * 1000)::bigint
                AS refresh_expires_at_ms
           FROM agent_room.device_refresh_token AS refresh
           JOIN agent_room.device_token_family AS family ON family.id = refresh.family_id
           JOIN agent_room.device AS device ON device.id = family.device_id
           JOIN agent_room.principal AS principal ON principal.id = device.principal_id
           WHERE refresh.secret_digest = $1
           FOR UPDATE OF refresh, family, device, principal"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(refresh_token_digest.as_bytes().as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| {
        Ok(LockedRefreshContext {
            account: decode_principal_account(&row, operation)?,
            device: decode_device(&row, operation)?,
            family: decode_token_family(&row, operation)?,
            refresh_id: decode(&row, "refresh_id", operation)?,
            sequence: decode(&row, "sequence", operation)?,
            consumed: decode(&row, "refresh_consumed", operation)?,
            revoked: decode(&row, "refresh_revoked", operation)?,
            expires_at: decode_time(&row, "refresh_expires_at_ms", operation)?,
        })
    })
    .transpose()
}

async fn persist_refresh_replacement(
    transaction: &mut Transaction<'_, Postgres>,
    locked: &LockedRefreshContext,
    replacement: &DeviceTokenReplacement,
    operation: &'static str,
) -> RepositoryResult<StoredDeviceSession> {
    let next_sequence = locked
        .sequence
        .checked_add(1)
        .ok_or_else(|| corrupt_data(operation))?;
    insert_access_token(
        transaction,
        locked.device.id(),
        locked.family.id(),
        replacement.access_token_id.as_uuid(),
        &replacement.access_token_digest,
        replacement.issued_at,
        replacement.access_token_expires_at,
        operation,
    )
    .await?;
    insert_refresh_token(
        transaction,
        locked.family.id(),
        replacement.refresh_token_id.as_uuid(),
        &replacement.refresh_token_digest,
        next_sequence,
        replacement.issued_at,
        locked.family.expires_at(),
        operation,
    )
    .await?;
    sqlx::query(
        r"UPDATE agent_room.device_refresh_token
           SET consumed_at = to_timestamp($2::double precision / 1000.0),
               replaced_by_id = $3
           WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(locked.refresh_id)
    .bind(replacement.issued_at.value())
    .bind(replacement.refresh_token_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(StoredDeviceSession {
        account: locked.account.clone(),
        device: locked.device.clone(),
        family: locked.family.clone(),
        access_token_expires_at: replacement.access_token_expires_at,
    })
}

async fn revoke_device_transaction(
    pool: &sqlx::PgPool,
    principal_id: PrincipalId,
    device_id: DeviceId,
    security_event: DeviceSecurityEvent,
) -> RepositoryResult<DeviceRevocationOutcome> {
    let operation = "device.revoke";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let query = format!(
        r"SELECT {DEVICE_SELECT}
           FROM agent_room.device AS device
           WHERE device.id = $1 AND device.principal_id = $2
           FOR UPDATE"
    );
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(device_id.as_uuid())
        .bind(principal_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?
    else {
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        return Ok(DeviceRevocationOutcome::NotFound);
    };
    let device = decode_device(&row, operation)?;
    let already_revoked = device.trust_state() == DeviceTrustState::Revoked;
    if !already_revoked {
        revoke_device_and_tokens(
            &mut transaction,
            device.id(),
            security_event.occurred_at,
            operation,
        )
        .await?;
        let event = device_event(
            security_event,
            device.id(),
            principal_id,
            "device.revoked.v1",
            "user_requested",
        )?;
        insert_outbox_event(&mut transaction, &event).await?;
    }
    let pending =
        pending_agent_matrix_device_revocations(&mut transaction, device.id(), operation).await?;
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    if already_revoked {
        Ok(DeviceRevocationOutcome::AlreadyRevoked(pending))
    } else {
        Ok(DeviceRevocationOutcome::Revoked(pending))
    }
}

async fn pending_agent_matrix_device_revocations(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    operation: &'static str,
) -> RepositoryResult<Vec<PendingAgentMatrixDeviceRevocation>> {
    let rows = sqlx::query(
        r"SELECT instance.id, agent.matrix_user_id, instance.matrix_device_id
           FROM agent_room.agent_instance AS instance
           JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
           WHERE instance.device_id = $1
             AND instance.revoked_at IS NOT NULL
             AND instance.matrix_device_revoked_at IS NULL
           ORDER BY instance.id",
    )
    .bind(device_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| {
            Ok(PendingAgentMatrixDeviceRevocation {
                instance_id: AgentInstanceId::from_uuid(
                    row.try_get::<Uuid, _>("id")
                        .map_err(|error| map_sqlx_error(operation, &error))?,
                ),
                matrix_user_id: row
                    .try_get("matrix_user_id")
                    .map_err(|error| map_sqlx_error(operation, &error))?,
                matrix_device_id: row
                    .try_get("matrix_device_id")
                    .map_err(|error| map_sqlx_error(operation, &error))?,
            })
        })
        .collect()
}

async fn compromise_device(
    transaction: &mut Transaction<'_, Postgres>,
    device: &Device,
    family_id: DeviceTokenFamilyId,
    security_event: DeviceSecurityEvent,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.device_token_family
           SET state = 'compromised',
               revoked_at = COALESCE(revoked_at, to_timestamp($2::double precision / 1000.0)),
               compromise_detected_at = to_timestamp($2::double precision / 1000.0)
           WHERE id = $1 AND state = 'active'",
    )
    .bind(family_id.as_uuid())
    .bind(security_event.occurred_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    revoke_device_and_tokens(
        transaction,
        device.id(),
        security_event.occurred_at,
        operation,
    )
    .await?;
    let event = device_event(
        security_event,
        device.id(),
        device.principal_id(),
        "device.compromised.v1",
        "refresh_token_reuse",
    )?;
    insert_outbox_event(transaction, &event).await
}

async fn insert_device_if_absent(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    device: &Device,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.device (
            id, principal_id, label, platform, public_signing_key, signing_algorithm,
            trust_state, verified_at, created_at, version
        ) VALUES (
            $1, $2, $3, $4, $5, 'ed25519', 'verified',
            to_timestamp($6::double precision / 1000.0),
            to_timestamp($7::double precision / 1000.0), 0
        ) ON CONFLICT (public_signing_key) WHERE revoked_at IS NULL DO NOTHING",
    )
    .bind(device.id().as_uuid())
    .bind(principal_id.as_uuid())
    .bind(device.label())
    .bind(device.platform().as_str())
    .bind(device.public_signing_key().as_bytes().as_slice())
    .bind(device.created_at().value())
    .bind(device.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn lock_device_by_active_key(
    transaction: &mut Transaction<'_, Postgres>,
    public_key: &DevicePublicSigningKey,
    operation: &'static str,
) -> RepositoryResult<Device> {
    let query = format!(
        r"SELECT {DEVICE_SELECT}
           FROM agent_room.device AS device
           WHERE device.public_signing_key = $1 AND device.revoked_at IS NULL
           FOR UPDATE"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(public_key.as_bytes().as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?
        .ok_or_else(|| corrupt_data(operation))?;
    decode_device(&row, operation)
}

async fn update_verified_device(
    transaction: &mut Transaction<'_, Postgres>,
    existing: &Device,
    requested: &Device,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.device
           SET label = $2, platform = $3, trust_state = 'verified',
               verified_at = COALESCE(verified_at, to_timestamp($4::double precision / 1000.0)),
               version = version + 1
           WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(existing.id().as_uuid())
    .bind(requested.label())
    .bind(requested.platform().as_str())
    .bind(requested.created_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn revoke_active_families(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    revoked_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    revoke_tokens(transaction, device_id, revoked_at, operation).await?;
    sqlx::query(
        r"UPDATE agent_room.device_token_family
           SET state = 'revoked', revoked_at = to_timestamp($2::double precision / 1000.0)
           WHERE device_id = $1 AND state = 'active'",
    )
    .bind(device_id.as_uuid())
    .bind(revoked_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn revoke_device_and_tokens(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    revoked_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.device
           SET trust_state = 'revoked',
               revoked_at = to_timestamp($2::double precision / 1000.0),
               version = version + 1
           WHERE id = $1 AND trust_state <> 'revoked'",
    )
    .bind(device_id.as_uuid())
    .bind(revoked_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    revoke_active_families(transaction, device_id, revoked_at, operation).await?;
    sqlx::query(
        r"UPDATE agent_room.agent_instance
           SET status = 'revoked',
               lease_expires_at = NULL,
               revoked_at = to_timestamp($2::double precision / 1000.0)
           WHERE device_id = $1 AND status <> 'revoked'",
    )
    .bind(device_id.as_uuid())
    .bind(revoked_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    fail_targeted_handoffs_for_device(transaction, device_id, revoked_at, operation).await?;
    Ok(())
}

async fn revoke_tokens(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    revoked_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.device_access_token
           SET revoked_at = to_timestamp($2::double precision / 1000.0)
           WHERE device_id = $1 AND revoked_at IS NULL",
    )
    .bind(device_id.as_uuid())
    .bind(revoked_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    sqlx::query(
        r"UPDATE agent_room.device_refresh_token AS refresh
           SET revoked_at = to_timestamp($2::double precision / 1000.0)
           FROM agent_room.device_token_family AS family
           WHERE refresh.family_id = family.id
             AND family.device_id = $1
             AND refresh.revoked_at IS NULL",
    )
    .bind(device_id.as_uuid())
    .bind(revoked_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn insert_token_family(
    transaction: &mut Transaction<'_, Postgres>,
    family: &DeviceTokenFamily,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.device_token_family (
            id, device_id, state, created_at, expires_at
        ) VALUES (
            $1, $2, $3,
            to_timestamp($4::double precision / 1000.0),
            to_timestamp($5::double precision / 1000.0)
        )",
    )
    .bind(family.id().as_uuid())
    .bind(family.device_id().as_uuid())
    .bind(family.state().as_str())
    .bind(family.created_at().value())
    .bind(family.expires_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_access_token(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    family_id: DeviceTokenFamilyId,
    token_id: Uuid,
    digest: &SecretDigest,
    issued_at: UtcMillis,
    expires_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.device_access_token (
            id, family_id, device_id, secret_digest, issued_at, expires_at
        ) VALUES (
            $1, $2, $3, $4,
            to_timestamp($5::double precision / 1000.0),
            to_timestamp($6::double precision / 1000.0)
        )",
    )
    .bind(token_id)
    .bind(family_id.as_uuid())
    .bind(device_id.as_uuid())
    .bind(digest.as_bytes().as_slice())
    .bind(issued_at.value())
    .bind(expires_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_refresh_token(
    transaction: &mut Transaction<'_, Postgres>,
    family_id: DeviceTokenFamilyId,
    token_id: Uuid,
    digest: &SecretDigest,
    sequence: i64,
    issued_at: UtcMillis,
    expires_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.device_refresh_token (
            id, family_id, secret_digest, sequence, issued_at, expires_at
        ) VALUES (
            $1, $2, $3, $4,
            to_timestamp($5::double precision / 1000.0),
            to_timestamp($6::double precision / 1000.0)
        )",
    )
    .bind(token_id)
    .bind(family_id.as_uuid())
    .bind(digest.as_bytes().as_slice())
    .bind(sequence)
    .bind(issued_at.value())
    .bind(expires_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_stored_device_session(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<StoredDeviceSession> {
    Ok(StoredDeviceSession {
        account: decode_principal_account(row, operation)?,
        device: decode_device(row, operation)?,
        family: decode_token_family(row, operation)?,
        access_token_expires_at: decode_time(row, "access_expires_at_ms", operation)?,
    })
}

fn decode_device(row: &PgRow, operation: &'static str) -> RepositoryResult<Device> {
    let id = DeviceId::from_uuid(decode(row, "device_id", operation)?);
    let principal_id = PrincipalId::from_uuid(decode(row, "device_principal_id", operation)?);
    let platform: String = decode(row, "device_platform", operation)?;
    let platform =
        DevicePlatform::try_from(platform.as_str()).map_err(|_| corrupt_data(operation))?;
    let trust_state: String = decode(row, "device_trust_state", operation)?;
    let trust_state =
        DeviceTrustState::try_from(trust_state.as_str()).map_err(|_| corrupt_data(operation))?;
    let public_key: Vec<u8> = decode(row, "public_signing_key", operation)?;
    let public_key =
        DevicePublicSigningKey::new(public_key).map_err(|_| corrupt_data(operation))?;
    Device::restore(
        id,
        principal_id,
        decode(row, "device_label", operation)?,
        platform,
        public_key,
        decode(row, "matrix_device_id", operation)?,
        trust_state,
        decode_optional_time(row, "device_last_seen_at_ms", operation)?,
        decode_optional_time(row, "device_revoked_at_ms", operation)?,
        decode_time(row, "device_created_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))
}

fn decode_token_family(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<DeviceTokenFamily> {
    let state: String = decode(row, "family_state", operation)?;
    let state =
        DeviceTokenFamilyState::try_from(state.as_str()).map_err(|_| corrupt_data(operation))?;
    DeviceTokenFamily::restore(
        DeviceTokenFamilyId::from_uuid(decode(row, "family_id", operation)?),
        DeviceId::from_uuid(decode(row, "family_device_id", operation)?),
        state,
        decode_time(row, "family_created_at_ms", operation)?,
        decode_time(row, "family_expires_at_ms", operation)?,
        decode_optional_time(row, "family_revoked_at_ms", operation)?,
        decode_optional_time(row, "family_compromise_detected_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))
}

fn device_event(
    security_event: DeviceSecurityEvent,
    device_id: DeviceId,
    principal_id: PrincipalId,
    event_type: &str,
    reason: &str,
) -> RepositoryResult<OutboxMessage> {
    let payload = Map::from_iter([
        ("deviceId".to_owned(), Value::String(device_id.to_string())),
        (
            "principalId".to_owned(),
            Value::String(principal_id.to_string()),
        ),
        ("reason".to_owned(), Value::String(reason.to_owned())),
    ]);
    OutboxMessage::new(
        security_event.id,
        "device".to_owned(),
        device_id.as_uuid(),
        event_type.to_owned(),
        payload,
        security_event.occurred_at,
    )
    .map_err(|_| corrupt_data("device.event"))
}

fn decode<T>(row: &PgRow, column: &str, operation: &'static str) -> RepositoryResult<T>
where
    for<'decode> T: sqlx::Decode<'decode, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))
}

fn decode_time(row: &PgRow, column: &str, operation: &'static str) -> RepositoryResult<UtcMillis> {
    let value: i64 = decode(row, column, operation)?;
    UtcMillis::new(value).map_err(|_| corrupt_data(operation))
}

fn decode_optional_time(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<Option<UtcMillis>> {
    let value: Option<i64> = decode(row, column, operation)?;
    value
        .map(UtcMillis::new)
        .transpose()
        .map_err(|_| corrupt_data(operation))
}

fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
