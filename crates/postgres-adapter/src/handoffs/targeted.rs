use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ClaimTargetedHandoff, PortFuture, QueueTargetedHandoff, QueueTargetedHandoffOutcome,
        RecordTargetedHandoffReceipt, TargetedHandoffReceiptOutcome, TargetedHandoffRepository,
        TargetedHandoffTargetRecord,
    },
};
use agent_room_domain::{
    agents::AgentInstanceStatus,
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    devices::DevicePlatform,
    handoff::{
        HandoffContentReference, HandoffFailureCode, HandoffPermission, HandoffPermissions,
        HandoffPurpose, HandoffSourceEventId, TargetedHandoff, TargetedHandoffFields,
        TargetedHandoffStatus,
    },
    ids::{AgentId, AgentInstanceId, ContentId, DeviceId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sqlx::{Executor, Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{corrupt_data, decode_column, decode_optional_time, decode_time},
    error::map_sqlx_error,
    transaction::finish,
};

const TARGETED_HANDOFF_CAPABILITY: &str = "targeted_handoff_v1";

const HANDOFF_COLUMNS: &str = r"handoff.id AS handoff_id,
       handoff.principal_id AS handoff_principal_id,
       handoff.source_matrix_room_id AS handoff_source_room_id,
       handoff.source_matrix_event_id AS handoff_source_event_id,
       handoff.source_message_id AS handoff_source_message_id,
       target.agent_id AS handoff_target_agent_id,
       handoff.target_agent_instance_id AS handoff_target_instance_id,
       content.id AS handoff_content_id,
       content.sha256_digest AS handoff_content_digest,
       content.byte_length AS handoff_content_byte_length,
       content.media_type AS handoff_content_media_type,
       handoff.permissions AS handoff_permissions,
       handoff.allowed_purpose AS handoff_purpose,
       handoff.state AS handoff_state,
       floor(extract(epoch FROM COALESCE(handoff.created_at, handoff.approved_at)) * 1000)::bigint
           AS handoff_created_at_ms,
       floor(extract(epoch FROM COALESCE(handoff.queued_at, handoff.approved_at)) * 1000)::bigint
           AS handoff_queued_at_ms,
       floor(extract(epoch FROM handoff.delivered_at) * 1000)::bigint
           AS handoff_delivered_at_ms,
       floor(extract(epoch FROM handoff.consumed_at) * 1000)::bigint
           AS handoff_consumed_at_ms,
       floor(extract(epoch FROM handoff.resolved_at) * 1000)::bigint
           AS handoff_resolved_at_ms,
       floor(extract(epoch FROM handoff.expires_at) * 1000)::bigint
           AS handoff_expires_at_ms,
       handoff.failure_code AS handoff_failure_code,
       handoff.version AS handoff_version";

const HANDOFF_FROM: &str = r"FROM agent_room.context_handoff AS handoff
       JOIN agent_room.agent_instance AS target
         ON target.id = handoff.target_agent_instance_id
       JOIN agent_room.content_object AS content ON content.id = handoff.content_id";

impl TargetedHandoffRepository for PostgresRepositories {
    fn list_targets(
        &self,
        principal_id: PrincipalId,
        _observed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<TargetedHandoffTargetRecord>>> {
        Box::pin(async move {
            const OPERATION: &str = "handoff.targets.list";
            let rows = sqlx::query(
                r"SELECT instance.id AS instance_id,
                          instance.agent_id,
                          agent.display_name AS agent_display_name,
                          agent.avatar_content_id AS agent_avatar_content_id,
                          instance.device_id,
                          device.label AS device_label,
                          device.platform AS device_platform,
                          instance.status AS instance_status,
                          floor(extract(epoch FROM instance.lease_expires_at) * 1000)::bigint
                              AS lease_expires_at_ms,
                          floor(extract(epoch FROM instance.last_seen_at) * 1000)::bigint
                              AS last_seen_at_ms,
                          binding.adapter_type,
                          binding.capability_version
                     FROM agent_room.agent_instance AS instance
                     JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
                     JOIN agent_room.adapter_binding AS binding
                       ON binding.id = instance.adapter_binding_id
                     JOIN agent_room.device AS device ON device.id = instance.device_id
                     JOIN agent_room.principal AS device_owner
                       ON device_owner.id = device.principal_id
                     JOIN agent_room.agent_ownership AS ownership
                       ON ownership.agent_id = instance.agent_id
                      AND ownership.principal_id = $1
                      AND ownership.revoked_at IS NULL
                      AND ownership.role IN ('owner', 'operator')
                    WHERE instance.revoked_at IS NULL
                      AND instance.status <> 'revoked'
                      AND agent.lifecycle_state = 'active'
                      AND binding.state = 'active'
                      AND binding.configuration @> jsonb_build_object(
                          'capabilities', jsonb_build_array($2::text)
                      )
                      AND device.revoked_at IS NULL
                      AND device.trust_state = 'verified'
                      AND device_owner.status = 'active'
                    ORDER BY agent.display_name, agent.id, device.label, instance.id",
            )
            .bind(principal_id.as_uuid())
            .bind(TARGETED_HANDOFF_CAPABILITY)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            rows.iter()
                .map(|row| decode_target(row, OPERATION))
                .collect()
        })
    }

    fn queue<'a>(
        &'a self,
        request: QueueTargetedHandoff<'a>,
    ) -> PortFuture<'a, RepositoryResult<QueueTargetedHandoffOutcome>> {
        Box::pin(async move { self.queue_targeted_handoff(request).await })
    }

    fn find_for_principal(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        observed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move {
            const OPERATION: &str = "handoff.find";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            let result = async {
                expire_handoff(&mut transaction, handoff_id, observed_at, OPERATION).await?;
                let handoff = load_handoff(&mut *transaction, handoff_id, OPERATION).await?;
                Ok(handoff.filter(|handoff| handoff.fields().principal_id == principal_id))
            }
            .await;
            finish(transaction, result, OPERATION).await
        })
    }

    fn revoke(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move {
            self.revoke_targeted_handoff(handoff_id, principal_id, revoked_at)
                .await
        })
    }

    fn claim_next(
        &self,
        request: ClaimTargetedHandoff,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move { self.claim_targeted_handoff(request).await })
    }

    fn record_receipt(
        &self,
        request: RecordTargetedHandoffReceipt,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move { self.record_targeted_handoff_receipt(request).await })
    }
}

async fn insert_targeted_handoff(
    transaction: &mut Transaction<'_, Postgres>,
    request: &QueueTargetedHandoff<'_>,
    byte_length: i64,
    permissions: Vec<String>,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let fields = request.handoff.fields();
    let inserted = sqlx::query_scalar::<_, uuid::Uuid>(
        r"INSERT INTO agent_room.context_handoff (
               id, principal_id, target_agent_instance_id,
               source_matrix_room_id, source_matrix_event_id, source_message_id,
               content_id, allowed_purpose, permissions, state,
               approved_at, created_at, queued_at, expires_at, request_fingerprint
           )
           SELECT $1, $2, $3, $5, $6, $7, $8, $12, $13, 'queued',
                  to_timestamp($15::double precision / 1000.0),
                  to_timestamp($15::double precision / 1000.0),
                  to_timestamp($15::double precision / 1000.0),
                  to_timestamp($16::double precision / 1000.0), $14
             FROM agent_room.agent_instance AS target
             JOIN agent_room.agent AS agent ON agent.id = target.agent_id
             JOIN agent_room.adapter_binding AS binding ON binding.id = target.adapter_binding_id
             JOIN agent_room.device AS device ON device.id = target.device_id
             JOIN agent_room.principal AS device_owner ON device_owner.id = device.principal_id
             JOIN agent_room.agent_ownership AS ownership
               ON ownership.agent_id = target.agent_id
              AND ownership.principal_id = $2
              AND ownership.revoked_at IS NULL
              AND ownership.role IN ('owner', 'operator')
             JOIN agent_room.content_object AS content ON content.id = $8
             JOIN agent_room.content_access_policy AS policy ON policy.content_id = content.id
            WHERE target.id = $3
              AND target.agent_id = $4
              AND target.revoked_at IS NULL
              AND target.status <> 'revoked'
              AND agent.lifecycle_state = 'active'
              AND binding.state = 'active'
              AND binding.configuration @> jsonb_build_object(
                  'capabilities', jsonb_build_array($17::text)
              )
              AND device.revoked_at IS NULL
              AND device.trust_state = 'verified'
              AND device_owner.status = 'active'
              AND content.lifecycle_state = 'active'
              AND content.sha256_digest = $9
              AND content.byte_length = $10
              AND content.media_type = $11
              AND (content.expires_at IS NULL OR content.expires_at >=
                  to_timestamp($16::double precision / 1000.0))
              AND policy.matrix_room_id = $5
              AND policy.matrix_event_id = $6
              AND policy.revoked_at IS NULL
           ON CONFLICT (id) DO NOTHING
           RETURNING id",
    )
    .bind(fields.id.as_uuid())
    .bind(fields.principal_id.as_uuid())
    .bind(fields.target_instance_id.as_uuid())
    .bind(fields.target_agent_id.as_uuid())
    .bind(fields.source_room_id.as_str())
    .bind(fields.source_event_id.as_str())
    .bind(fields.source_message_id.as_uuid())
    .bind(fields.content.content_id().as_uuid())
    .bind(fields.content.digest().as_bytes().as_slice())
    .bind(byte_length)
    .bind(fields.content.media_type().as_str())
    .bind(fields.purpose.as_str())
    .bind(permissions)
    .bind(request.request_fingerprint.as_bytes().as_slice())
    .bind(fields.created_at.value())
    .bind(fields.expires_at.value())
    .bind(TARGETED_HANDOFF_CAPABILITY)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(inserted.is_some())
}

impl PostgresRepositories {
    async fn queue_targeted_handoff(
        &self,
        request: QueueTargetedHandoff<'_>,
    ) -> RepositoryResult<QueueTargetedHandoffOutcome> {
        const OPERATION: &str = "handoff.queue";
        let fields = request.handoff.fields();
        let byte_length = i64::try_from(fields.content.byte_length().value())
            .map_err(|_| RepositoryError::new(OPERATION, RepositoryErrorKind::Constraint))?;
        let permissions = fields
            .permissions
            .iter()
            .map(|permission| permission.as_str().to_owned())
            .collect::<Vec<_>>();
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = async {
            let inserted = insert_targeted_handoff(
                &mut transaction,
                &request,
                byte_length,
                permissions,
                OPERATION,
            )
            .await?;

            let stored = load_handoff(&mut *transaction, fields.id, OPERATION)
                .await?
                .ok_or_else(|| RepositoryError::new(OPERATION, RepositoryErrorKind::Forbidden))?;
            if !inserted {
                let stored_fingerprint = sqlx::query_scalar::<_, Vec<u8>>(
                    "SELECT request_fingerprint FROM agent_room.context_handoff WHERE id = $1",
                )
                .bind(fields.id.as_uuid())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(OPERATION, &error))?;
                if stored_fingerprint.as_slice() != request.request_fingerprint.as_bytes() {
                    return Err(RepositoryError::new(
                        OPERATION,
                        RepositoryErrorKind::Conflict,
                    ));
                }
                return Ok(QueueTargetedHandoffOutcome::Existing(stored));
            }
            Ok(QueueTargetedHandoffOutcome::Created(stored))
        }
        .await;
        finish(transaction, result, OPERATION).await
    }

    async fn revoke_targeted_handoff(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        revoked_at: UtcMillis,
    ) -> RepositoryResult<Option<TargetedHandoff>> {
        const OPERATION: &str = "handoff.revoke";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = async {
            expire_handoff(&mut transaction, handoff_id, revoked_at, OPERATION).await?;
            let changed = sqlx::query_scalar::<_, uuid::Uuid>(
                r"UPDATE agent_room.context_handoff
                      SET state = 'revoked', resolved_at = to_timestamp($3::double precision / 1000.0),
                          failure_code = NULL, version = version + 1
                    WHERE id = $1 AND principal_id = $2 AND state IN ('queued', 'delivered')
                    RETURNING id",
            )
            .bind(handoff_id.as_uuid())
            .bind(principal_id.as_uuid())
            .bind(revoked_at.value())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            let handoff = load_handoff(&mut *transaction, handoff_id, OPERATION)
                .await?
                .filter(|handoff| handoff.fields().principal_id == principal_id);
            match (changed, handoff) {
                (_, None) => Ok(None),
                (Some(_), Some(handoff)) => Ok(Some(handoff)),
                (None, Some(handoff)) if handoff.status() == TargetedHandoffStatus::Revoked => {
                    Ok(Some(handoff))
                }
                (None, Some(_)) => Err(RepositoryError::new(
                    OPERATION,
                    RepositoryErrorKind::Conflict,
                )),
            }
        }
        .await;
        finish(transaction, result, OPERATION).await
    }

    async fn claim_targeted_handoff(
        &self,
        request: ClaimTargetedHandoff,
    ) -> RepositoryResult<Option<TargetedHandoff>> {
        const OPERATION: &str = "handoff.claim";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = async {
            expire_handoffs_for_instance(
                &mut transaction,
                request.target_instance_id,
                request.claimed_at,
                OPERATION,
            )
            .await?;
            let claimed = sqlx::query_scalar::<_, uuid::Uuid>(
                r"WITH candidate AS (
                       SELECT handoff.id
                         FROM agent_room.context_handoff AS handoff
                         JOIN agent_room.agent_instance AS target
                           ON target.id = handoff.target_agent_instance_id
                         JOIN agent_room.agent AS agent ON agent.id = target.agent_id
                         JOIN agent_room.adapter_binding AS binding
                           ON binding.id = target.adapter_binding_id
                         JOIN agent_room.device AS device ON device.id = target.device_id
                         JOIN agent_room.principal AS device_owner
                           ON device_owner.id = device.principal_id
                         JOIN agent_room.agent_ownership AS creator_access
                           ON creator_access.agent_id = target.agent_id
                          AND creator_access.principal_id = handoff.principal_id
                          AND creator_access.revoked_at IS NULL
                          AND creator_access.role IN ('owner', 'operator')
                        WHERE target.id = $1
                          AND target.device_id = $2
                          AND device.principal_id = $3
                          AND handoff.state = 'queued'
                          AND handoff.expires_at > to_timestamp($4::double precision / 1000.0)
                          AND target.revoked_at IS NULL
                          AND target.status <> 'revoked'
                          AND agent.lifecycle_state = 'active'
                          AND binding.state = 'active'
                          AND binding.configuration @> jsonb_build_object(
                              'capabilities', jsonb_build_array($5::text)
                          )
                          AND device.revoked_at IS NULL
                          AND device.trust_state = 'verified'
                          AND device_owner.status = 'active'
                        ORDER BY handoff.queued_at, handoff.id
                        FOR UPDATE OF handoff SKIP LOCKED
                        LIMIT 1
                   )
                   UPDATE agent_room.context_handoff AS handoff
                      SET state = 'delivered',
                          delivered_at = to_timestamp($4::double precision / 1000.0),
                          version = version + 1
                     FROM candidate
                    WHERE handoff.id = candidate.id
                    RETURNING handoff.id",
            )
            .bind(request.target_instance_id.as_uuid())
            .bind(request.device_id.as_uuid())
            .bind(request.principal_id.as_uuid())
            .bind(request.claimed_at.value())
            .bind(TARGETED_HANDOFF_CAPABILITY)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            match claimed {
                Some(id) => {
                    load_handoff(&mut *transaction, HandoffId::from_uuid(id), OPERATION).await
                }
                None => Ok(None),
            }
        }
        .await;
        finish(transaction, result, OPERATION).await
    }

    async fn record_targeted_handoff_receipt(
        &self,
        request: RecordTargetedHandoffReceipt,
    ) -> RepositoryResult<Option<TargetedHandoff>> {
        const OPERATION: &str = "handoff.receipt";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
        let result = async {
            expire_handoff(
                &mut transaction,
                request.handoff_id,
                request.recorded_at,
                OPERATION,
            )
            .await?;
            let current = sqlx::query(
                r"SELECT handoff.state, handoff.failure_code
                     FROM agent_room.context_handoff AS handoff
                     JOIN agent_room.agent_instance AS target
                       ON target.id = handoff.target_agent_instance_id
                     JOIN agent_room.device AS device ON device.id = target.device_id
                     JOIN agent_room.principal AS device_owner
                       ON device_owner.id = device.principal_id
                     JOIN agent_room.agent_ownership AS creator_access
                       ON creator_access.agent_id = target.agent_id
                      AND creator_access.principal_id = handoff.principal_id
                      AND creator_access.revoked_at IS NULL
                      AND creator_access.role IN ('owner', 'operator')
                    WHERE handoff.id = $1
                      AND target.id = $2
                      AND target.device_id = $3
                      AND device.principal_id = $4
                      AND target.revoked_at IS NULL
                      AND target.status <> 'revoked'
                      AND device.revoked_at IS NULL
                      AND device.trust_state = 'verified'
                      AND device_owner.status = 'active'
                    FOR UPDATE OF handoff",
            )
            .bind(request.handoff_id.as_uuid())
            .bind(request.target_instance_id.as_uuid())
            .bind(request.device_id.as_uuid())
            .bind(request.principal_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            let Some(current) = current else {
                return Ok(None);
            };
            let current_state: String = decode_column(&current, "state", OPERATION)?;
            let current_failure: Option<String> =
                decode_column(&current, "failure_code", OPERATION)?;
            let (target_state, failure_code) = receipt_state(&request.outcome);
            if current_state == target_state
                && current_failure.as_deref() == failure_code.as_deref()
            {
                return load_handoff(&mut *transaction, request.handoff_id, OPERATION).await;
            }
            if current_state != "delivered" {
                return Err(RepositoryError::new(
                    OPERATION,
                    RepositoryErrorKind::Conflict,
                ));
            }
            let consumed_at = (target_state == "consumed").then_some(request.recorded_at.value());
            sqlx::query(
                r"UPDATE agent_room.context_handoff
                      SET state = $2,
                          consumed_at = CASE WHEN $2 = 'consumed'
                              THEN to_timestamp($3::double precision / 1000.0)
                              ELSE NULL END,
                          resolved_at = to_timestamp($3::double precision / 1000.0),
                          failure_code = $4,
                          version = version + 1
                    WHERE id = $1",
            )
            .bind(request.handoff_id.as_uuid())
            .bind(target_state)
            .bind(consumed_at.unwrap_or(request.recorded_at.value()))
            .bind(failure_code)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(OPERATION, &error))?;
            load_handoff(&mut *transaction, request.handoff_id, OPERATION).await
        }
        .await;
        finish(transaction, result, OPERATION).await
    }
}

fn decode_target(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<TargetedHandoffTargetRecord> {
    let platform: String = decode_column(row, "device_platform", operation)?;
    let status: String = decode_column(row, "instance_status", operation)?;
    Ok(TargetedHandoffTargetRecord {
        instance_id: AgentInstanceId::from_uuid(decode_column(row, "instance_id", operation)?),
        agent_id: AgentId::from_uuid(decode_column(row, "agent_id", operation)?),
        agent_display_name: decode_column(row, "agent_display_name", operation)?,
        agent_avatar_content_id: decode_column::<Option<uuid::Uuid>>(
            row,
            "agent_avatar_content_id",
            operation,
        )?
        .map(ContentId::from_uuid),
        device_id: DeviceId::from_uuid(decode_column(row, "device_id", operation)?),
        device_label: decode_column(row, "device_label", operation)?,
        device_platform: DevicePlatform::try_from(platform.as_str())
            .map_err(|_| corrupt_data(operation))?,
        instance_status: AgentInstanceStatus::try_from(status.as_str())
            .map_err(|_| corrupt_data(operation))?,
        lease_expires_at: decode_optional_time(row, "lease_expires_at_ms", operation)?,
        last_seen_at: decode_optional_time(row, "last_seen_at_ms", operation)?,
        adapter_type: decode_column(row, "adapter_type", operation)?,
        capability_version: decode_column(row, "capability_version", operation)?,
    })
}

async fn load_handoff<'e, E>(
    executor: E,
    handoff_id: HandoffId,
    operation: &'static str,
) -> RepositoryResult<Option<TargetedHandoff>>
where
    E: Executor<'e, Database = Postgres>,
{
    let statement = format!(
        "SELECT {HANDOFF_COLUMNS} {HANDOFF_FROM} WHERE handoff.id = $1 AND handoff.source_message_id IS NOT NULL"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(handoff_id.as_uuid())
        .fetch_optional(executor)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_handoff(row, operation))
        .transpose()
}

fn decode_handoff(row: &PgRow, operation: &'static str) -> RepositoryResult<TargetedHandoff> {
    let digest: Vec<u8> = decode_column(row, "handoff_content_digest", operation)?;
    let digest: [u8; 32] = digest.try_into().map_err(|_| corrupt_data(operation))?;
    let byte_length: i64 = decode_column(row, "handoff_content_byte_length", operation)?;
    let byte_length = u64::try_from(byte_length).map_err(|_| corrupt_data(operation))?;
    let permissions: Vec<String> = decode_column(row, "handoff_permissions", operation)?;
    let permissions = permissions
        .iter()
        .map(|permission| {
            HandoffPermission::try_from(permission.as_str()).map_err(|_| corrupt_data(operation))
        })
        .collect::<RepositoryResult<Vec<_>>>()?;
    let status: String = decode_column(row, "handoff_state", operation)?;
    let status = decode_status(&status).ok_or_else(|| corrupt_data(operation))?;
    let failure_code: Option<String> = decode_column(row, "handoff_failure_code", operation)?;
    let version: i64 = decode_column(row, "handoff_version", operation)?;
    let version = u64::try_from(version).map_err(|_| corrupt_data(operation))?;
    TargetedHandoff::restore(
        TargetedHandoffFields {
            id: HandoffId::from_uuid(decode_column(row, "handoff_id", operation)?),
            principal_id: PrincipalId::from_uuid(decode_column(
                row,
                "handoff_principal_id",
                operation,
            )?),
            source_room_id: MatrixRoomReference::new(decode_column::<String>(
                row,
                "handoff_source_room_id",
                operation,
            )?)
            .map_err(|_| corrupt_data(operation))?,
            source_event_id: HandoffSourceEventId::new(decode_column::<String>(
                row,
                "handoff_source_event_id",
                operation,
            )?)
            .map_err(|_| corrupt_data(operation))?,
            source_message_id: MessageId::from_uuid(decode_column(
                row,
                "handoff_source_message_id",
                operation,
            )?),
            target_agent_id: AgentId::from_uuid(decode_column(
                row,
                "handoff_target_agent_id",
                operation,
            )?),
            target_instance_id: AgentInstanceId::from_uuid(decode_column(
                row,
                "handoff_target_instance_id",
                operation,
            )?),
            content: HandoffContentReference::new(
                ContentId::from_uuid(decode_column(row, "handoff_content_id", operation)?),
                Sha256Digest::from_bytes(digest),
                ContentByteLength::new(byte_length).map_err(|_| corrupt_data(operation))?,
                ContentMediaType::new(decode_column::<String>(
                    row,
                    "handoff_content_media_type",
                    operation,
                )?)
                .map_err(|_| corrupt_data(operation))?,
            ),
            permissions: HandoffPermissions::new(permissions)
                .map_err(|_| corrupt_data(operation))?,
            purpose: HandoffPurpose::try_from(
                decode_column::<String>(row, "handoff_purpose", operation)?.as_str(),
            )
            .map_err(|_| corrupt_data(operation))?,
            created_at: decode_time(row, "handoff_created_at_ms", operation)?,
            expires_at: decode_time(row, "handoff_expires_at_ms", operation)?,
        },
        status,
        decode_time(row, "handoff_queued_at_ms", operation)?,
        decode_optional_time(row, "handoff_delivered_at_ms", operation)?,
        decode_optional_time(row, "handoff_consumed_at_ms", operation)?,
        decode_optional_time(row, "handoff_resolved_at_ms", operation)?,
        failure_code
            .map(HandoffFailureCode::new)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        version,
    )
    .map_err(|_| corrupt_data(operation))
}

const fn decode_status(value: &str) -> Option<TargetedHandoffStatus> {
    match value.as_bytes() {
        b"approved" | b"queued" => Some(TargetedHandoffStatus::Queued),
        b"delivered" => Some(TargetedHandoffStatus::Delivered),
        b"consumed" => Some(TargetedHandoffStatus::Consumed),
        b"declined" => Some(TargetedHandoffStatus::Declined),
        b"revoked" => Some(TargetedHandoffStatus::Revoked),
        b"expired" => Some(TargetedHandoffStatus::Expired),
        b"failed" => Some(TargetedHandoffStatus::Failed),
        _ => None,
    }
}

async fn expire_handoff(
    transaction: &mut Transaction<'_, Postgres>,
    handoff_id: HandoffId,
    observed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.context_handoff
              SET state = 'expired',
                  resolved_at = to_timestamp($2::double precision / 1000.0),
                  failure_code = NULL,
                  version = version + 1
            WHERE id = $1
              AND state IN ('queued', 'delivered')
              AND expires_at <= to_timestamp($2::double precision / 1000.0)",
    )
    .bind(handoff_id.as_uuid())
    .bind(observed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn expire_handoffs_for_instance(
    transaction: &mut Transaction<'_, Postgres>,
    instance_id: AgentInstanceId,
    observed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.context_handoff
              SET state = 'expired',
                  resolved_at = to_timestamp($2::double precision / 1000.0),
                  failure_code = NULL,
                  version = version + 1
            WHERE target_agent_instance_id = $1
              AND state IN ('queued', 'delivered')
              AND expires_at <= to_timestamp($2::double precision / 1000.0)",
    )
    .bind(instance_id.as_uuid())
    .bind(observed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn receipt_state(outcome: &TargetedHandoffReceiptOutcome) -> (&'static str, Option<String>) {
    match outcome {
        TargetedHandoffReceiptOutcome::Consumed => ("consumed", None),
        TargetedHandoffReceiptOutcome::Declined(code) => {
            ("declined", Some(code.as_str().to_owned()))
        }
        TargetedHandoffReceiptOutcome::Failed(code) => ("failed", Some(code.as_str().to_owned())),
    }
}

pub(crate) async fn fail_targeted_handoffs_for_instance(
    transaction: &mut Transaction<'_, Postgres>,
    instance_id: AgentInstanceId,
    failed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.context_handoff
              SET state = CASE
                      WHEN expires_at <= to_timestamp($2::double precision / 1000.0)
                          THEN 'expired'
                      ELSE 'failed'
                  END,
                  resolved_at = to_timestamp($2::double precision / 1000.0),
                  failure_code = CASE
                      WHEN expires_at <= to_timestamp($2::double precision / 1000.0)
                          THEN NULL
                      ELSE 'handoff.target_revoked'
                  END,
                  version = version + 1
            WHERE target_agent_instance_id = $1
              AND source_message_id IS NOT NULL
              AND state IN ('queued', 'delivered')",
    )
    .bind(instance_id.as_uuid())
    .bind(failed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

pub(crate) async fn fail_targeted_handoffs_for_device(
    transaction: &mut Transaction<'_, Postgres>,
    device_id: DeviceId,
    failed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.context_handoff AS handoff
              SET state = CASE
                      WHEN handoff.expires_at <= to_timestamp($2::double precision / 1000.0)
                          THEN 'expired'
                      ELSE 'failed'
                  END,
                  resolved_at = to_timestamp($2::double precision / 1000.0),
                  failure_code = CASE
                      WHEN handoff.expires_at <= to_timestamp($2::double precision / 1000.0)
                          THEN NULL
                      ELSE 'handoff.target_device_revoked'
                  END,
                  version = handoff.version + 1
             FROM agent_room.agent_instance AS target
            WHERE target.id = handoff.target_agent_instance_id
              AND target.device_id = $1
              AND handoff.source_message_id IS NOT NULL
              AND handoff.state IN ('queued', 'delivered')",
    )
    .bind(device_id.as_uuid())
    .bind(failed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}
