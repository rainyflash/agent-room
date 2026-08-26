use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AccountDeletionClaim, AccountDeletionRepository, AccountDeletionRequest,
        AccountDeletionRequestOutcome, AccountDeletionStage, AccountDeletionStatus,
        AccountExportSnapshot, MatrixUserId, PortFuture, SecretDigest,
    },
};
use agent_room_domain::{
    ids::{AccountDeletionJobId, PrincipalId},
    time::UtcMillis,
};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use crate::{PostgresRepositories, agents::decode_time, error::map_sqlx_error};

const FIND_BY_RECEIPT_SQL: &str = r"SELECT id, stage, attempt_count,
    floor(extract(epoch FROM requested_at) * 1000)::bigint AS requested_at_ms,
    floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
    floor(extract(epoch FROM retry_at) * 1000)::bigint AS retry_at_ms,
    floor(extract(epoch FROM completed_at) * 1000)::bigint AS completed_at_ms,
    failure_code
    FROM agent_room.account_deletion_job WHERE receipt_digest = $1";

const FIND_BY_PRINCIPAL_SQL: &str = r"SELECT id, stage, attempt_count,
    floor(extract(epoch FROM requested_at) * 1000)::bigint AS requested_at_ms,
    floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
    floor(extract(epoch FROM retry_at) * 1000)::bigint AS retry_at_ms,
    floor(extract(epoch FROM completed_at) * 1000)::bigint AS completed_at_ms,
    failure_code
    FROM agent_room.account_deletion_job WHERE principal_id = $1 FOR UPDATE";

const INSERT_DELETION_SQL: &str = r"INSERT INTO agent_room.account_deletion_job (
    id, principal_id, matrix_user_id, receipt_digest, requested_at, updated_at
) VALUES (
    $1, $2, $3, $4,
    to_timestamp($5::double precision / 1000.0),
    to_timestamp($5::double precision / 1000.0)
) RETURNING id, stage, attempt_count,
    floor(extract(epoch FROM requested_at) * 1000)::bigint AS requested_at_ms,
    floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
    floor(extract(epoch FROM retry_at) * 1000)::bigint AS retry_at_ms,
    floor(extract(epoch FROM completed_at) * 1000)::bigint AS completed_at_ms,
    failure_code";

const COMPLETE_DELETION_SQL: &str = r"UPDATE agent_room.account_deletion_job
    SET stage = 'completed', retry_at = NULL, lease_expires_at = NULL,
        failure_code = NULL, completed_at = to_timestamp($3::double precision / 1000.0),
        updated_at = to_timestamp($3::double precision / 1000.0), version = version + 1
    WHERE id = $1 AND version = $2 AND stage = 'local_erasure'
    RETURNING id, stage, attempt_count,
        floor(extract(epoch FROM requested_at) * 1000)::bigint AS requested_at_ms,
        floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
        floor(extract(epoch FROM retry_at) * 1000)::bigint AS retry_at_ms,
        floor(extract(epoch FROM completed_at) * 1000)::bigint AS completed_at_ms,
        failure_code";

impl AccountDeletionRepository for PostgresRepositories {
    fn export(
        &self,
        principal_id: PrincipalId,
        generated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AccountExportSnapshot>>> {
        Box::pin(export_account(&self.pool, principal_id, generated_at))
    }

    fn request<'a>(
        &'a self,
        request: &'a AccountDeletionRequest,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionRequestOutcome>> {
        Box::pin(request_deletion(&self.pool, request))
    }

    fn find_by_receipt<'a>(
        &'a self,
        receipt_digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<AccountDeletionStatus>>> {
        Box::pin(async move {
            const OPERATION: &str = "account_deletion.find_by_receipt";
            sqlx::query(FIND_BY_RECEIPT_SQL)
                .bind(receipt_digest.as_bytes().as_slice())
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?
                .map(|row| decode_status(&row, OPERATION))
                .transpose()
        })
    }

    fn claim_due(
        &self,
        now: UtcMillis,
        lease_expires_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AccountDeletionClaim>>> {
        Box::pin(claim_due(&self.pool, now, lease_expires_at))
    }

    fn record_federated_deactivation<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionClaim>> {
        Box::pin(record_federated_deactivation(
            &self.pool,
            claim,
            completed_at,
        ))
    }

    fn schedule_retry<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        failure_code: &'a str,
        retry_at: UtcMillis,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(schedule_retry(
            &self.pool,
            claim,
            failure_code,
            retry_at,
            changed_at,
        ))
    }

    fn finalize_local<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionStatus>> {
        Box::pin(finalize_local(&self.pool, claim, completed_at))
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "单一 JSON 导出投影的声明式 SQL 保持在同一处更容易审计字段边界"
)]
async fn export_account(
    pool: &sqlx::PgPool,
    principal_id: PrincipalId,
    generated_at: UtcMillis,
) -> RepositoryResult<Option<AccountExportSnapshot>> {
    const OPERATION: &str = "account_export.read";
    let data: Option<Value> = sqlx::query_scalar(
        r"SELECT jsonb_build_object(
            'principal', jsonb_build_object(
                'id', principal.id,
                'matrixUserId', principal.matrix_user_id,
                'displayName', principal.display_name,
                'locale', principal.locale,
                'status', principal.status,
                'createdAt', principal.created_at,
                'updatedAt', principal.updated_at
            ),
            'devices', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', device.id,
                    'label', device.label,
                    'platform', device.platform,
                    'matrixDeviceId', device.matrix_device_id,
                    'trustState', device.trust_state,
                    'lastSeenAt', device.last_seen_at,
                    'createdAt', device.created_at,
                    'revokedAt', device.revoked_at
                ) ORDER BY device.created_at)
                FROM agent_room.device AS device
                WHERE device.principal_id = principal.id
            ), '[]'::jsonb),
            'agents', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', agent.id,
                    'matrixUserId', agent.matrix_user_id,
                    'slug', agent.slug,
                    'displayName', agent.display_name,
                    'description', agent.description,
                    'visibility', agent.visibility,
                    'lifecycleState', agent.lifecycle_state,
                    'role', ownership.role,
                    'createdAt', agent.created_at,
                    'updatedAt', agent.updated_at
                ) ORDER BY agent.created_at)
                FROM agent_room.agent_ownership AS ownership
                JOIN agent_room.agent AS agent ON agent.id = ownership.agent_id
                WHERE ownership.principal_id = principal.id
            ), '[]'::jsonb),
            'rooms', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', room.id,
                    'kind', room.kind,
                    'name', room.name,
                    'description', room.description,
                    'visibility', room.visibility,
                    'retentionDays', room.retention_days,
                    'status', room.status,
                    'createdAt', room.created_at,
                    'updatedAt', room.updated_at
                ) ORDER BY room.created_at)
                FROM agent_room.room_catalog_entry AS room
                WHERE room.owner_principal_id = principal.id
                   OR EXISTS (
                       SELECT 1
                       FROM agent_room.private_room_membership AS membership
                       WHERE membership.catalog_entry_id = room.id
                         AND membership.principal_id = principal.id
                   )
            ), '[]'::jsonb),
            'content', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', content.id,
                    'sha256', encode(content.sha256_digest, 'hex'),
                    'byteLength', content.byte_length,
                    'mediaType', content.media_type,
                    'encryptionMode', content.encryption_mode,
                    'lifecycleState', content.lifecycle_state,
                    'expiresAt', content.expires_at,
                    'createdAt', content.created_at,
                    'deletedAt', content.deleted_at
                ) ORDER BY content.created_at)
                FROM agent_room.content_object AS content
                WHERE content.owner_principal_id = principal.id
            ), '[]'::jsonb),
            'moderationCases', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'id', moderation.id,
                    'targetKind', moderation.target_kind,
                    'targetReference', moderation.target_reference,
                    'reasonCode', moderation.reason_code,
                    'description', moderation.description,
                    'state', moderation.state,
                    'createdAt', moderation.created_at,
                    'resolvedAt', moderation.resolved_at
                ) ORDER BY moderation.created_at)
                FROM agent_room.moderation_case AS moderation
                WHERE moderation.reporter_principal_id = principal.id
            ), '[]'::jsonb),
            'externalData', jsonb_build_object(
                'matrixTimeline', 'export_with_matrix_client_before_deletion',
                'federatedCopies', 'controlled_by_receiving_homeservers_and_recipients',
                'auditTombstones', 'retained_without_message_bodies'
            )
        )
        FROM agent_room.principal AS principal
        WHERE principal.id = $1 AND principal.status IN ('active', 'suspended')",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    Ok(data.map(|data| AccountExportSnapshot {
        schema_version: 1,
        generated_at,
        data,
    }))
}

async fn request_deletion(
    pool: &sqlx::PgPool,
    request: &AccountDeletionRequest,
) -> RepositoryResult<AccountDeletionRequestOutcome> {
    const OPERATION: &str = "account_deletion.request";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if let Some(row) = existing_deletion(&mut transaction, request.principal_id, OPERATION).await? {
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        return decode_status(&row, OPERATION).map(AccountDeletionRequestOutcome::Existing);
    }

    let principal_row = sqlx::query(
        "SELECT status, matrix_user_id FROM agent_room.principal WHERE id = $1 FOR UPDATE",
    )
    .bind(request.principal_id.as_uuid())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?
    .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::NotFound))?;
    let status: String = principal_row
        .try_get("status")
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let matrix_user_id: String = principal_row
        .try_get("matrix_user_id")
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if status != "active" || matrix_user_id != request.matrix_user_id.as_str() {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }

    let changed_at = request.requested_at.value();
    let result = sqlx::query(
        r"UPDATE agent_room.principal
          SET status = 'deleting', updated_at = to_timestamp($2::double precision / 1000.0),
              version = version + 1
          WHERE id = $1",
    )
    .bind(request.principal_id.as_uuid())
    .bind(changed_at)
    .execute(&mut *transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if result.rows_affected() != 1 {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }
    revoke_local_credentials(
        &mut transaction,
        request.principal_id,
        changed_at,
        OPERATION,
    )
    .await?;

    let row = sqlx::query(INSERT_DELETION_SQL)
        .bind(request.job_id.as_uuid())
        .bind(request.principal_id.as_uuid())
        .bind(request.matrix_user_id.as_str())
        .bind(request.receipt_digest.as_bytes().as_slice())
        .bind(changed_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let result = decode_status(&row, OPERATION)?;
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    Ok(AccountDeletionRequestOutcome::Created(result))
}

async fn existing_deletion(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    operation: &'static str,
) -> RepositoryResult<Option<PgRow>> {
    sqlx::query(FIND_BY_PRINCIPAL_SQL)
        .bind(principal_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))
}

async fn revoke_local_credentials(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    changed_at: i64,
    operation: &'static str,
) -> RepositoryResult<()> {
    let statements = [
        r"UPDATE agent_room.web_session
          SET revoked_at = COALESCE(revoked_at, to_timestamp($2::double precision / 1000.0))
          WHERE principal_id = $1 AND revoked_at IS NULL",
        r"UPDATE agent_room.device_token_family AS family
          SET state = 'revoked', revoked_at = to_timestamp($2::double precision / 1000.0)
          FROM agent_room.device AS device
          WHERE family.device_id = device.id AND device.principal_id = $1 AND family.state = 'active'",
        r"UPDATE agent_room.device_access_token AS token
          SET revoked_at = COALESCE(token.revoked_at, to_timestamp($2::double precision / 1000.0))
          FROM agent_room.device AS device
          WHERE token.device_id = device.id AND device.principal_id = $1 AND token.revoked_at IS NULL",
        r"UPDATE agent_room.device_refresh_token AS token
          SET revoked_at = COALESCE(token.revoked_at, to_timestamp($2::double precision / 1000.0))
          FROM agent_room.device_token_family AS family
          JOIN agent_room.device AS device ON device.id = family.device_id
          WHERE token.family_id = family.id AND device.principal_id = $1 AND token.revoked_at IS NULL",
        r"UPDATE agent_room.device
          SET trust_state = 'revoked', revoked_at = to_timestamp($2::double precision / 1000.0),
              version = version + 1
          WHERE principal_id = $1 AND revoked_at IS NULL",
        r"UPDATE agent_room.agent_instance AS instance
          SET status = 'revoked', lease_expires_at = NULL,
              revoked_at = to_timestamp($2::double precision / 1000.0)
          FROM agent_room.device AS device
          WHERE instance.device_id = device.id AND device.principal_id = $1
            AND instance.revoked_at IS NULL",
        r"UPDATE agent_room.automation_grant
          SET state = 'revoked', revoked_at = to_timestamp($2::double precision / 1000.0),
              version = version + 1
          WHERE principal_id = $1 AND state = 'active'",
    ];
    for statement in statements {
        sqlx::query(statement)
            .bind(principal_id.as_uuid())
            .bind(changed_at)
            .execute(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
    }
    Ok(())
}

async fn claim_due(
    pool: &sqlx::PgPool,
    now: UtcMillis,
    lease_expires_at: UtcMillis,
) -> RepositoryResult<Option<AccountDeletionClaim>> {
    const OPERATION: &str = "account_deletion.claim_due";
    let row = sqlx::query(
        r"WITH candidate AS (
            SELECT id
            FROM agent_room.account_deletion_job
            WHERE (
                stage = 'queued'
                OR (stage = 'retry_scheduled' AND retry_at <= to_timestamp($1::double precision / 1000.0))
                OR stage = 'local_erasure'
                OR (
                    stage = 'federated_deactivation'
                    AND lease_expires_at <= to_timestamp($1::double precision / 1000.0)
                )
            )
            ORDER BY requested_at, id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE agent_room.account_deletion_job AS job
        SET stage = CASE
                WHEN job.stage = 'local_erasure' THEN 'local_erasure'
                ELSE 'federated_deactivation'
            END,
            attempt_count = job.attempt_count + 1,
            retry_at = NULL,
            lease_expires_at = CASE
                WHEN job.stage = 'local_erasure' THEN NULL
                ELSE to_timestamp($2::double precision / 1000.0)
            END,
            failure_code = NULL,
            updated_at = to_timestamp($1::double precision / 1000.0),
            version = job.version + 1
        FROM candidate
        WHERE job.id = candidate.id
        RETURNING job.id, job.principal_id, job.matrix_user_id, job.stage,
                  job.attempt_count, job.version",
    )
    .bind(now.value())
    .bind(lease_expires_at.value())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    row.map(|row| decode_claim(&row, OPERATION)).transpose()
}

async fn record_federated_deactivation(
    pool: &sqlx::PgPool,
    claim: &AccountDeletionClaim,
    completed_at: UtcMillis,
) -> RepositoryResult<AccountDeletionClaim> {
    const OPERATION: &str = "account_deletion.record_federated_deactivation";
    let row = sqlx::query(
        r"UPDATE agent_room.account_deletion_job
          SET stage = 'local_erasure', lease_expires_at = NULL, failure_code = NULL,
              updated_at = to_timestamp($3::double precision / 1000.0), version = version + 1
          WHERE id = $1 AND version = $2 AND stage = 'federated_deactivation'
          RETURNING id, principal_id, matrix_user_id, stage, attempt_count, version",
    )
    .bind(claim.job_id.as_uuid())
    .bind(claim.version)
    .bind(completed_at.value())
    .fetch_optional(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?
    .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Conflict))?;
    decode_claim(&row, OPERATION)
}

async fn schedule_retry(
    pool: &sqlx::PgPool,
    claim: &AccountDeletionClaim,
    failure_code: &str,
    retry_at: UtcMillis,
    changed_at: UtcMillis,
) -> RepositoryResult<()> {
    const OPERATION: &str = "account_deletion.schedule_retry";
    let result = sqlx::query(
        r"UPDATE agent_room.account_deletion_job
          SET stage = 'retry_scheduled', retry_at = to_timestamp($3::double precision / 1000.0),
              lease_expires_at = NULL, failure_code = $4,
              updated_at = to_timestamp($5::double precision / 1000.0), version = version + 1
          WHERE id = $1 AND version = $2 AND stage = 'federated_deactivation'",
    )
    .bind(claim.job_id.as_uuid())
    .bind(claim.version)
    .bind(retry_at.value())
    .bind(failure_code)
    .bind(changed_at.value())
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ))
    }
}

async fn finalize_local(
    pool: &sqlx::PgPool,
    claim: &AccountDeletionClaim,
    completed_at: UtcMillis,
) -> RepositoryResult<AccountDeletionStatus> {
    const OPERATION: &str = "account_deletion.finalize_local";
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let changed_at = completed_at.value();
    retire_exclusive_agents(&mut transaction, claim.principal_id, changed_at, OPERATION).await?;
    archive_private_rooms(&mut transaction, claim.principal_id, changed_at, OPERATION).await?;
    redact_local_content(&mut transaction, claim.principal_id, changed_at, OPERATION).await?;
    revoke_local_credentials(&mut transaction, claim.principal_id, changed_at, OPERATION).await?;

    let statements = [
        r"UPDATE agent_room.agent_ownership
          SET revoked_at = COALESCE(revoked_at, to_timestamp($2::double precision / 1000.0))
          WHERE principal_id = $1 AND revoked_at IS NULL",
        r"UPDATE agent_room.private_room_membership
          SET membership_status = 'removed', permission_bits = 0,
              status_changed_at = to_timestamp($2::double precision / 1000.0)
          WHERE principal_id = $1
            AND catalog_entry_id NOT IN (
                SELECT id FROM agent_room.room_catalog_entry
                WHERE owner_principal_id = $1 AND kind = 'private_room'
            )
            AND membership_status IN ('invited', 'joined')",
        r"UPDATE agent_room.direct_contact_block
          SET revoked_at = COALESCE(revoked_at, to_timestamp($2::double precision / 1000.0))
          WHERE principal_id = $1 AND revoked_at IS NULL",
        r"UPDATE agent_room.context_handoff
          SET state = 'revoked', failure_code = NULL, version = version + 1
          WHERE principal_id = $1 AND state IN ('proposed', 'approved', 'delivered')",
        r"UPDATE agent_room.moderation_case
          SET description = '', reporter_submitted_excerpt = NULL
          WHERE reporter_principal_id = $1",
    ];
    for statement in statements {
        sqlx::query(statement)
            .bind(claim.principal_id.as_uuid())
            .bind(changed_at)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    }

    let principal_result = sqlx::query(
        r"UPDATE agent_room.principal
          SET oidc_issuer = 'urn:agent-room:deleted',
              oidc_subject = 'deleted:' || $2::text,
              display_name = 'Deleted account', avatar_content_id = NULL, locale = 'en',
              status = 'deleted', updated_at = to_timestamp($3::double precision / 1000.0),
              version = version + 1
          WHERE id = $1 AND status = 'deleting'",
    )
    .bind(claim.principal_id.as_uuid())
    .bind(claim.job_id.as_uuid())
    .bind(changed_at)
    .execute(&mut *transaction)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if principal_result.rows_affected() != 1 {
        return Err(RepositoryError::new(
            OPERATION,
            RepositoryErrorKind::Conflict,
        ));
    }

    let row = sqlx::query(COMPLETE_DELETION_SQL)
        .bind(claim.job_id.as_uuid())
        .bind(claim.version)
        .bind(changed_at)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?
        .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Conflict))?;
    let status = decode_status(&row, OPERATION)?;
    transaction
        .commit()
        .await
        .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    Ok(status)
}

async fn retire_exclusive_agents(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    changed_at: i64,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"WITH exclusive_agents AS (
            SELECT ownership.agent_id
            FROM agent_room.agent_ownership AS ownership
            WHERE ownership.principal_id = $1
              AND ownership.role = 'owner'
              AND ownership.revoked_at IS NULL
              AND NOT EXISTS (
                  SELECT 1
                  FROM agent_room.agent_ownership AS other_owner
                  WHERE other_owner.agent_id = ownership.agent_id
                    AND other_owner.principal_id <> $1
                    AND other_owner.role = 'owner'
                    AND other_owner.revoked_at IS NULL
              )
        )
        UPDATE agent_room.agent AS agent
        SET lifecycle_state = 'retired', display_name = 'Deleted agent', description = '',
            avatar_content_id = NULL, visibility = 'private',
            updated_at = to_timestamp($2::double precision / 1000.0), version = version + 1
        FROM exclusive_agents
        WHERE agent.id = exclusive_agents.agent_id",
    )
    .bind(principal_id.as_uuid())
    .bind(changed_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn archive_private_rooms(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    changed_at: i64,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.room_instance AS instance
          SET state = 'archived', updated_at = to_timestamp($2::double precision / 1000.0),
              version = version + 1
          FROM agent_room.room_catalog_entry AS room
          WHERE instance.catalog_entry_id = room.id
            AND room.owner_principal_id = $1 AND room.kind = 'private_room'",
    )
    .bind(principal_id.as_uuid())
    .bind(changed_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    sqlx::query(
        r"UPDATE agent_room.room_catalog_entry
          SET status = 'archived', name = 'Deleted room', description = '',
              updated_at = to_timestamp($2::double precision / 1000.0)
          WHERE owner_principal_id = $1 AND kind = 'private_room'",
    )
    .bind(principal_id.as_uuid())
    .bind(changed_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn redact_local_content(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    changed_at: i64,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.content_access_policy AS policy
          SET revoked_at = COALESCE(policy.revoked_at, to_timestamp($2::double precision / 1000.0)),
              updated_at = to_timestamp($2::double precision / 1000.0)
          FROM agent_room.content_object AS content
          WHERE policy.content_id = content.id AND content.owner_principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .bind(changed_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    sqlx::query(
        r"UPDATE agent_room.content_object
          SET lifecycle_state = CASE
                  WHEN lifecycle_state = 'uploading' THEN 'orphaned'
                  WHEN lifecycle_state = 'active' THEN 'redacted'
                  ELSE lifecycle_state
              END,
              updated_at = to_timestamp($2::double precision / 1000.0), version = version + 1
          WHERE owner_principal_id = $1
            AND lifecycle_state IN ('uploading', 'active')",
    )
    .bind(principal_id.as_uuid())
    .bind(changed_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_status(row: &PgRow, operation: &'static str) -> RepositoryResult<AccountDeletionStatus> {
    let attempt_count: i32 = row
        .try_get("attempt_count")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(AccountDeletionStatus {
        job_id: AccountDeletionJobId::from_uuid(
            row.try_get("id")
                .map_err(|error| map_sqlx_error(operation, &error))?,
        ),
        stage: decode_stage(row, operation)?,
        attempt_count: u16::try_from(attempt_count)
            .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))?,
        requested_at: decode_time(row, "requested_at_ms", operation)?,
        updated_at: decode_time(row, "updated_at_ms", operation)?,
        retry_at: decode_optional_time(row, "retry_at_ms", operation)?,
        completed_at: decode_optional_time(row, "completed_at_ms", operation)?,
        failure_code: row
            .try_get("failure_code")
            .map_err(|error| map_sqlx_error(operation, &error))?,
    })
}

fn decode_claim(row: &PgRow, operation: &'static str) -> RepositoryResult<AccountDeletionClaim> {
    let attempt_count: i32 = row
        .try_get("attempt_count")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let matrix_user_id: String = row
        .try_get("matrix_user_id")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(AccountDeletionClaim {
        job_id: AccountDeletionJobId::from_uuid(
            row.try_get("id")
                .map_err(|error| map_sqlx_error(operation, &error))?,
        ),
        principal_id: PrincipalId::from_uuid(
            row.try_get("principal_id")
                .map_err(|error| map_sqlx_error(operation, &error))?,
        ),
        matrix_user_id: MatrixUserId::new(matrix_user_id)
            .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))?,
        stage: decode_stage(row, operation)?,
        attempt_count: u16::try_from(attempt_count)
            .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))?,
        version: row
            .try_get("version")
            .map_err(|error| map_sqlx_error(operation, &error))?,
    })
}

fn decode_stage(row: &PgRow, operation: &'static str) -> RepositoryResult<AccountDeletionStage> {
    let stage: String = row
        .try_get("stage")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    match stage.as_str() {
        "queued" => Ok(AccountDeletionStage::Queued),
        "federated_deactivation" => Ok(AccountDeletionStage::FederatedDeactivation),
        "local_erasure" => Ok(AccountDeletionStage::LocalErasure),
        "retry_scheduled" => Ok(AccountDeletionStage::RetryScheduled),
        "completed" => Ok(AccountDeletionStage::Completed),
        _ => Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::CorruptData,
        )),
    }
}

fn decode_optional_time(
    row: &PgRow,
    column: &str,
    operation: &'static str,
) -> RepositoryResult<Option<UtcMillis>> {
    let value: Option<i64> = row
        .try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))?;
    value
        .map(UtcMillis::new)
        .transpose()
        .map_err(|_| RepositoryError::new(operation, RepositoryErrorKind::CorruptData))
}
