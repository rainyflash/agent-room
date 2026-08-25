use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentInstanceManagementRecord, AgentInstanceManagementRepository,
        AgentInstanceMatrixCleanupStore, AgentInstanceRegistration,
        AgentInstanceRegistrationTransaction, AgentInstanceRevocationOutcome,
        AgentInstanceRevocationTransaction, AgentInstanceVerificationRecord,
        AgentInstanceVerificationRepository, OutboxMessage, PortFuture, SecretDigest,
        StoredAgentInstanceRegistration,
    },
};
use agent_room_domain::{
    agents::{
        AdapterBinding, AdapterBindingState, AdapterSubjectHash, AgentInstance,
        AgentInstancePublicSigningKey, AgentInstanceStatus, AgentMatrixDeviceId,
    },
    devices::{DevicePlatform, DeviceTrustState},
    ids::{AdapterBindingId, AgentId, AgentInstanceId, ContentId, DeviceId, PrincipalId},
    time::UtcMillis,
};
use serde_json::{Map, Value};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_column, decode_optional_time, decode_time},
    error::map_sqlx_error,
    outbox::insert_outbox_event,
};

const MANAGEMENT_SELECT: &str = r"instance.id, instance.agent_id, instance.device_id,
       instance.adapter_binding_id, instance.public_signing_key, instance.matrix_device_id,
       instance.status,
       floor(extract(epoch FROM instance.lease_expires_at) * 1000)::bigint
           AS lease_expires_at_ms,
       agent.matrix_user_id AS agent_matrix_user_id,
       agent.display_name AS agent_display_name,
       agent.avatar_content_id AS agent_avatar_content_id,
       binding.adapter_type AS adapter_type,
       binding.capability_version AS capability_version,
       device.label AS device_label,
       device.platform AS device_platform,
       device.trust_state AS device_trust_state,
       floor(extract(epoch FROM instance.created_at) * 1000)::bigint AS instance_created_at_ms,
       floor(extract(epoch FROM instance.last_seen_at) * 1000)::bigint AS last_seen_at_ms,
       floor(extract(epoch FROM instance.revoked_at) * 1000)::bigint AS revoked_at_ms,
       floor(extract(epoch FROM instance.matrix_device_revoked_at) * 1000)::bigint
           AS matrix_device_revoked_at_ms";

const MANAGEMENT_FROM: &str = r"FROM agent_room.agent_instance AS instance
       JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
       JOIN agent_room.adapter_binding AS binding ON binding.id = instance.adapter_binding_id
       JOIN agent_room.device AS device ON device.id = instance.device_id
       JOIN agent_room.agent_ownership AS ownership ON ownership.agent_id = instance.agent_id
       JOIN agent_room.principal AS principal ON principal.id = ownership.principal_id";

impl AgentInstanceRegistrationTransaction for PostgresRepositories {
    fn register_with_event<'a>(
        &'a self,
        registration: &'a AgentInstanceRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<StoredAgentInstanceRegistration>> {
        Box::pin(async move { self.register_agent_instance(registration, event).await })
    }
}

impl AgentInstanceVerificationRepository for PostgresRepositories {
    fn find_verification_record(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceVerificationRecord>>> {
        Box::pin(async move {
            let operation = "agent_instance.verification.find";
            let row = sqlx::query(
                r"SELECT instance.id, instance.agent_id, instance.public_signing_key,
                         floor(extract(epoch FROM instance.created_at) * 1000)::bigint
                             AS registered_at_ms,
                         floor(extract(epoch FROM (
                             SELECT min(invalidated_at)
                             FROM unnest(ARRAY[
                                 instance.revoked_at,
                                 device.revoked_at,
                                 CASE WHEN device.trust_state = 'verified'
                                     THEN NULL
                                     ELSE coalesce(device.revoked_at, instance.created_at)
                                 END,
                                 CASE WHEN principal.status = 'active'
                                     THEN NULL ELSE principal.updated_at END,
                                 CASE WHEN agent.lifecycle_state = 'active'
                                     THEN NULL ELSE agent.updated_at END,
                                 CASE WHEN binding.state = 'active'
                                     THEN NULL ELSE binding.updated_at END
                             ]) AS invalidations(invalidated_at)
                         )) * 1000)::bigint AS invalidated_at_ms
                  FROM agent_room.agent_instance AS instance
                  JOIN agent_room.device AS device ON device.id = instance.device_id
                  JOIN agent_room.principal AS principal ON principal.id = device.principal_id
                  JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
                  JOIN agent_room.adapter_binding AS binding
                    ON binding.id = instance.adapter_binding_id
                  WHERE instance.id = $1",
            )
            .bind(instance_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_verification_record(&row, operation))
                .transpose()
        })
    }
}

impl AgentInstanceManagementRepository for PostgresRepositories {
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentInstanceManagementRecord>>> {
        Box::pin(async move {
            let operation = "agent_instance.management.list";
            let query = format!(
                r"SELECT {MANAGEMENT_SELECT}
                   {MANAGEMENT_FROM}
                   WHERE ownership.principal_id = $1
                     AND ownership.revoked_at IS NULL
                     AND ownership.role IN ('owner', 'operator')
                     AND principal.status = 'active'
                   ORDER BY (instance.revoked_at IS NOT NULL),
                            instance.last_seen_at DESC NULLS LAST,
                            instance.created_at DESC,
                            instance.id"
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(principal_id.as_uuid())
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter()
                .map(|row| decode_management_record(row, operation))
                .collect()
        })
    }
}

impl AgentInstanceRevocationTransaction for PostgresRepositories {
    fn revoke<'a>(
        &'a self,
        principal_id: PrincipalId,
        instance_id: AgentInstanceId,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentInstanceRevocationOutcome>> {
        Box::pin(async move {
            self.revoke_agent_instance(principal_id, instance_id, event)
                .await
        })
    }
}

impl AgentInstanceMatrixCleanupStore for PostgresRepositories {
    fn mark_matrix_device_revoked(
        &self,
        instance_id: AgentInstanceId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "agent_instance.matrix_cleanup.complete";
            let result = sqlx::query(
                r"UPDATE agent_room.agent_instance
                   SET matrix_device_revoked_at = COALESCE(
                       matrix_device_revoked_at,
                       to_timestamp($2::double precision / 1000.0)
                   )
                   WHERE id = $1 AND revoked_at IS NOT NULL",
            )
            .bind(instance_id.as_uuid())
            .bind(revoked_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if result.rows_affected() == 1 {
                Ok(())
            } else {
                Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Constraint,
                ))
            }
        })
    }
}

impl PostgresRepositories {
    async fn register_agent_instance(
        &self,
        registration: &AgentInstanceRegistration,
        event: &OutboxMessage,
    ) -> RepositoryResult<StoredAgentInstanceRegistration> {
        let operation = "agent_instance.register";
        ensure_registration_coherence(registration)?;
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            if let Some(receipt) =
                lock_registration_receipt(&mut transaction, registration.request_id).await?
            {
                ensure_receipt_matches(&receipt, registration)?;
                return load_stored_registration(
                    &mut transaction,
                    receipt.binding_id,
                    receipt.instance_id,
                    "agent_instance.register.replay",
                )
                .await;
            }

            authorize_registration(&mut transaction, registration).await?;
            let binding = reconcile_binding(&mut transaction, registration).await?;
            let (instance, inserted) =
                reconcile_instance(&mut transaction, registration, &binding).await?;
            insert_registration_receipt(&mut transaction, registration, &binding, &instance)
                .await?;
            if inserted {
                ensure_event_contract(&instance, event)?;
                insert_outbox_event(&mut transaction, event).await?;
            }
            Ok(StoredAgentInstanceRegistration { binding, instance })
        }
        .await;

        crate::transaction::finish(transaction, result, operation).await
    }

    async fn revoke_agent_instance(
        &self,
        principal_id: PrincipalId,
        instance_id: AgentInstanceId,
        event: &OutboxMessage,
    ) -> RepositoryResult<AgentInstanceRevocationOutcome> {
        let operation = "agent_instance.management.revoke";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = async {
            let query = format!(
                r"SELECT {MANAGEMENT_SELECT}
                   {MANAGEMENT_FROM}
                   WHERE instance.id = $1
                     AND ownership.principal_id = $2
                     AND ownership.revoked_at IS NULL
                     AND ownership.role IN ('owner', 'operator')
                     AND principal.status = 'active'
                   FOR UPDATE OF instance, ownership"
            );
            let Some(row) = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(instance_id.as_uuid())
                .bind(principal_id.as_uuid())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?
            else {
                return Ok(AgentInstanceRevocationOutcome::NotFound);
            };
            let mut record = decode_management_record(&row, operation)?;
            if record.instance.status() == AgentInstanceStatus::Revoked {
                return Ok(AgentInstanceRevocationOutcome::AlreadyRevoked(record));
            }
            ensure_revocation_event_contract(instance_id, event)?;
            let update = sqlx::query(
                r"UPDATE agent_room.agent_instance
                   SET status = 'revoked',
                       lease_expires_at = NULL,
                       revoked_at = to_timestamp($2::double precision / 1000.0)
                   WHERE id = $1 AND status <> 'revoked'",
            )
            .bind(instance_id.as_uuid())
            .bind(event.occurred_at().value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            if update.rows_affected() != 1 {
                return Err(corrupt_data(operation));
            }
            insert_outbox_event(&mut transaction, event).await?;
            record.instance.revoke();
            record.revoked_at = Some(event.occurred_at());
            Ok(AgentInstanceRevocationOutcome::Revoked(record))
        }
        .await;
        crate::transaction::finish(transaction, result, operation).await
    }
}

#[derive(Debug, Clone, Copy)]
struct RegistrationReceipt {
    principal_id: PrincipalId,
    device_id: DeviceId,
    agent_id: AgentId,
    binding_id: AdapterBindingId,
    instance_id: AgentInstanceId,
    request_fingerprint: SecretDigest,
}

async fn lock_registration_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    request_id: agent_room_domain::ids::AgentInstanceRegistrationRequestId,
) -> RepositoryResult<Option<RegistrationReceipt>> {
    let operation = "agent_instance.register.receipt";
    let row = sqlx::query(
        r"SELECT principal_id, device_id, agent_id, adapter_binding_id,
               agent_instance_id, request_fingerprint
          FROM agent_room.agent_instance_registration_request
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(request_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| decode_receipt(&row, operation)).transpose()
}

fn decode_receipt(row: &PgRow, operation: &'static str) -> RepositoryResult<RegistrationReceipt> {
    let principal_id: uuid::Uuid = decode_column(row, "principal_id", operation)?;
    let device_id: uuid::Uuid = decode_column(row, "device_id", operation)?;
    let agent_id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    let binding_id: uuid::Uuid = decode_column(row, "adapter_binding_id", operation)?;
    let instance_id: uuid::Uuid = decode_column(row, "agent_instance_id", operation)?;
    let fingerprint: Vec<u8> = decode_column(row, "request_fingerprint", operation)?;
    let fingerprint = <[u8; 32]>::try_from(fingerprint).map_err(|_| corrupt_data(operation))?;
    Ok(RegistrationReceipt {
        principal_id: PrincipalId::from_uuid(principal_id),
        device_id: DeviceId::from_uuid(device_id),
        agent_id: AgentId::from_uuid(agent_id),
        binding_id: AdapterBindingId::from_uuid(binding_id),
        instance_id: AgentInstanceId::from_uuid(instance_id),
        request_fingerprint: SecretDigest::from_array(fingerprint),
    })
}

fn ensure_receipt_matches(
    receipt: &RegistrationReceipt,
    registration: &AgentInstanceRegistration,
) -> RepositoryResult<()> {
    if receipt.principal_id != registration.principal_id
        || receipt.device_id != registration.device_id
    {
        return Err(RepositoryError::new(
            "agent_instance.register.replay",
            RepositoryErrorKind::Forbidden,
        ));
    }
    if receipt.agent_id != registration.instance.agent_id()
        || receipt.request_fingerprint != registration.request_fingerprint
    {
        return Err(conflict("agent_instance.register.replay"));
    }
    Ok(())
}

async fn authorize_registration(
    transaction: &mut Transaction<'_, Postgres>,
    registration: &AgentInstanceRegistration,
) -> RepositoryResult<()> {
    let operation = "agent_instance.register.authorize";
    let agent_state: Option<String> =
        sqlx::query_scalar("SELECT lifecycle_state FROM agent_room.agent WHERE id = $1 FOR UPDATE")
            .bind(registration.instance.agent_id().as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
    if agent_state.as_deref() != Some("active") {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ));
    }

    let role: Option<String> = sqlx::query_scalar(
        r"SELECT role
          FROM agent_room.agent_ownership
          WHERE agent_id = $1 AND principal_id = $2 AND revoked_at IS NULL
          FOR UPDATE",
    )
    .bind(registration.instance.agent_id().as_uuid())
    .bind(registration.principal_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if !matches!(role.as_deref(), Some("owner" | "operator")) {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ));
    }

    let device = sqlx::query(
        r"SELECT device.principal_id, device.trust_state,
               device.revoked_at IS NOT NULL AS is_revoked,
               principal.status AS principal_status
          FROM agent_room.device
          JOIN agent_room.principal ON principal.id = device.principal_id
          WHERE device.id = $1
          FOR UPDATE OF device, principal",
    )
    .bind(registration.device_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| RepositoryError::new(operation, RepositoryErrorKind::Forbidden))?;
    let owner: uuid::Uuid = decode_column(&device, "principal_id", operation)?;
    let trust_state: String = decode_column(&device, "trust_state", operation)?;
    let is_revoked: bool = decode_column(&device, "is_revoked", operation)?;
    let principal_status: String = decode_column(&device, "principal_status", operation)?;
    if owner != registration.principal_id.as_uuid()
        || trust_state != "verified"
        || is_revoked
        || principal_status != "active"
    {
        return Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ));
    }
    Ok(())
}

async fn reconcile_binding(
    transaction: &mut Transaction<'_, Postgres>,
    registration: &AgentInstanceRegistration,
) -> RepositoryResult<AdapterBinding> {
    let binding = &registration.binding;
    let subject_hash = binding
        .external_subject_hash()
        .map(|hash| hash.as_bytes().as_slice());
    let existing = sqlx::query(
        r"SELECT id, agent_id, adapter_type, external_subject_hash,
               capability_version, configuration::text AS configuration_json, state
          FROM agent_room.adapter_binding
          WHERE agent_id = $1
            AND adapter_type = $2
            AND external_subject_hash IS NOT DISTINCT FROM $3::bytea
          FOR UPDATE",
    )
    .bind(binding.agent_id().as_uuid())
    .bind(binding.adapter_type())
    .bind(subject_hash)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_instance.register.binding", &error))?;

    if let Some(row) = existing {
        let (stored, configuration) = decode_binding(&row, "agent_instance.register.binding")?;
        if stored.capability_version() != binding.capability_version()
            || stored.state() != binding.state()
            || configuration != registration.binding_configuration
        {
            return Err(conflict("agent_instance.register.binding"));
        }
        return Ok(stored);
    }

    let configuration = serde_json::to_string(&registration.binding_configuration)
        .map_err(|_| corrupt_data("agent_instance.register.binding.serialize"))?;
    sqlx::query(
        r"INSERT INTO agent_room.adapter_binding (
            id, agent_id, adapter_type, external_subject_hash, capability_version,
            configuration, state, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6::jsonb, $7,
            to_timestamp($8::double precision / 1000.0),
            to_timestamp($8::double precision / 1000.0)
        )",
    )
    .bind(binding.id().as_uuid())
    .bind(binding.agent_id().as_uuid())
    .bind(binding.adapter_type())
    .bind(subject_hash)
    .bind(binding.capability_version())
    .bind(configuration)
    .bind(binding.state().as_str())
    .bind(registration.registered_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_instance.register.binding", &error))?;
    Ok(binding.clone())
}

async fn reconcile_instance(
    transaction: &mut Transaction<'_, Postgres>,
    registration: &AgentInstanceRegistration,
    binding: &AdapterBinding,
) -> RepositoryResult<(AgentInstance, bool)> {
    let proposed = &registration.instance;
    let rows = sqlx::query(
        r"SELECT id, agent_id, device_id, adapter_binding_id, public_signing_key,
               matrix_device_id, status,
               floor(extract(epoch FROM lease_expires_at) * 1000)::bigint AS lease_expires_at_ms
          FROM agent_room.agent_instance
          WHERE revoked_at IS NULL AND (
              public_signing_key = $1
              OR (agent_id = $2 AND device_id = $3 AND adapter_binding_id = $4)
          )
          FOR UPDATE",
    )
    .bind(proposed.public_signing_key().as_bytes().as_slice())
    .bind(proposed.agent_id().as_uuid())
    .bind(proposed.device_id().as_uuid())
    .bind(binding.id().as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_instance.register.instance", &error))?;

    match rows.as_slice() {
        [] => {
            insert_instance(transaction, proposed, binding, registration.registered_at).await?;
            Ok((proposed.clone(), true))
        }
        [row] => {
            let stored = decode_instance(row, "agent_instance.register.instance")?;
            if stored.agent_id() != proposed.agent_id()
                || stored.device_id() != proposed.device_id()
                || stored.adapter_binding_id() != binding.id()
                || stored.public_signing_key() != proposed.public_signing_key()
            {
                return Err(conflict("agent_instance.register.instance"));
            }
            Ok((stored, false))
        }
        _ => Err(corrupt_data("agent_instance.register.instance")),
    }
}

async fn insert_instance(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &AgentInstance,
    binding: &AdapterBinding,
    registered_at: UtcMillis,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance (
            id, agent_id, device_id, adapter_binding_id, public_signing_key,
            matrix_device_id, status, lease_expires_at, created_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, NULL,
            to_timestamp($8::double precision / 1000.0)
        )",
    )
    .bind(instance.id().as_uuid())
    .bind(instance.agent_id().as_uuid())
    .bind(instance.device_id().as_uuid())
    .bind(binding.id().as_uuid())
    .bind(instance.public_signing_key().as_bytes().as_slice())
    .bind(instance.matrix_device_id().as_str())
    .bind(instance.status().as_str())
    .bind(registered_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_instance.register.instance", &error))?;
    Ok(())
}

async fn insert_registration_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    registration: &AgentInstanceRegistration,
    binding: &AdapterBinding,
    instance: &AgentInstance,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance_registration_request (
            id, principal_id, device_id, agent_id, adapter_binding_id,
            agent_instance_id, request_fingerprint, created_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7,
            to_timestamp($8::double precision / 1000.0)
        )",
    )
    .bind(registration.request_id.as_uuid())
    .bind(registration.principal_id.as_uuid())
    .bind(registration.device_id.as_uuid())
    .bind(instance.agent_id().as_uuid())
    .bind(binding.id().as_uuid())
    .bind(instance.id().as_uuid())
    .bind(registration.request_fingerprint.as_bytes().as_slice())
    .bind(registration.registered_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_instance.register.receipt", &error))?;
    Ok(())
}

async fn load_stored_registration(
    transaction: &mut Transaction<'_, Postgres>,
    binding_id: AdapterBindingId,
    instance_id: AgentInstanceId,
    operation: &'static str,
) -> RepositoryResult<StoredAgentInstanceRegistration> {
    let binding_row = sqlx::query(
        r"SELECT id, agent_id, adapter_type, external_subject_hash,
               capability_version, configuration::text AS configuration_json, state
          FROM agent_room.adapter_binding WHERE id = $1",
    )
    .bind(binding_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| corrupt_data(operation))?;
    let (binding, _) = decode_binding(&binding_row, operation)?;

    let instance_row = sqlx::query(
        r"SELECT id, agent_id, device_id, adapter_binding_id, public_signing_key,
               matrix_device_id, status,
               floor(extract(epoch FROM lease_expires_at) * 1000)::bigint AS lease_expires_at_ms
          FROM agent_room.agent_instance WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .ok_or_else(|| corrupt_data(operation))?;
    let instance = decode_instance(&instance_row, operation)?;
    Ok(StoredAgentInstanceRegistration { binding, instance })
}

fn decode_binding(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<(AdapterBinding, Map<String, Value>)> {
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let agent_id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    let subject_hash: Option<Vec<u8>> = decode_column(row, "external_subject_hash", operation)?;
    let subject_hash = subject_hash
        .map(AdapterSubjectHash::new)
        .transpose()
        .map_err(|_| corrupt_data(operation))?;
    let state: String = decode_column(row, "state", operation)?;
    let state =
        AdapterBindingState::try_from(state.as_str()).map_err(|_| corrupt_data(operation))?;
    let binding = AdapterBinding::restore(
        AdapterBindingId::from_uuid(id),
        AgentId::from_uuid(agent_id),
        decode_column(row, "adapter_type", operation)?,
        subject_hash,
        decode_column(row, "capability_version", operation)?,
        state,
    )
    .map_err(|_| corrupt_data(operation))?;
    let configuration_json: String = decode_column(row, "configuration_json", operation)?;
    let configuration: Value =
        serde_json::from_str(&configuration_json).map_err(|_| corrupt_data(operation))?;
    let configuration = configuration
        .as_object()
        .cloned()
        .ok_or_else(|| corrupt_data(operation))?;
    Ok((binding, configuration))
}

fn decode_instance(row: &PgRow, operation: &'static str) -> RepositoryResult<AgentInstance> {
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let agent_id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    let device_id: uuid::Uuid = decode_column(row, "device_id", operation)?;
    let binding_id: uuid::Uuid = decode_column(row, "adapter_binding_id", operation)?;
    let signing_key: Vec<u8> = decode_column(row, "public_signing_key", operation)?;
    let status: String = decode_column(row, "status", operation)?;
    AgentInstance::restore(
        AgentInstanceId::from_uuid(id),
        AgentId::from_uuid(agent_id),
        DeviceId::from_uuid(device_id),
        AdapterBindingId::from_uuid(binding_id),
        AgentInstancePublicSigningKey::new(signing_key).map_err(|_| corrupt_data(operation))?,
        AgentMatrixDeviceId::new(decode_column(row, "matrix_device_id", operation)?)
            .map_err(|_| corrupt_data(operation))?,
        AgentInstanceStatus::try_from(status.as_str()).map_err(|_| corrupt_data(operation))?,
        decode_optional_time(row, "lease_expires_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))
}

fn decode_verification_record(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<AgentInstanceVerificationRecord> {
    let instance_id: uuid::Uuid = decode_column(row, "id", operation)?;
    let agent_id: uuid::Uuid = decode_column(row, "agent_id", operation)?;
    let signing_key: Vec<u8> = decode_column(row, "public_signing_key", operation)?;
    let registered_at_ms: i64 = decode_column(row, "registered_at_ms", operation)?;
    Ok(AgentInstanceVerificationRecord {
        instance_id: AgentInstanceId::from_uuid(instance_id),
        agent_id: AgentId::from_uuid(agent_id),
        public_signing_key: AgentInstancePublicSigningKey::new(signing_key)
            .map_err(|_| corrupt_data(operation))?,
        registered_at: UtcMillis::new(registered_at_ms).map_err(|_| corrupt_data(operation))?,
        invalidated_at: decode_optional_time(row, "invalidated_at_ms", operation)?,
    })
}

fn decode_management_record(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<AgentInstanceManagementRecord> {
    let avatar_content_id: Option<uuid::Uuid> =
        decode_column(row, "agent_avatar_content_id", operation)?;
    let platform: String = decode_column(row, "device_platform", operation)?;
    let trust_state: String = decode_column(row, "device_trust_state", operation)?;
    let record = AgentInstanceManagementRecord {
        instance: decode_instance(row, operation)?,
        agent_matrix_user_id: decode_column(row, "agent_matrix_user_id", operation)?,
        agent_display_name: decode_column(row, "agent_display_name", operation)?,
        agent_avatar_content_id: avatar_content_id.map(ContentId::from_uuid),
        adapter_type: decode_column(row, "adapter_type", operation)?,
        capability_version: decode_column(row, "capability_version", operation)?,
        device_label: decode_column(row, "device_label", operation)?,
        device_platform: DevicePlatform::try_from(platform.as_str())
            .map_err(|_| corrupt_data(operation))?,
        device_trust_state: DeviceTrustState::try_from(trust_state.as_str())
            .map_err(|_| corrupt_data(operation))?,
        created_at: decode_time(row, "instance_created_at_ms", operation)?,
        last_seen_at: decode_optional_time(row, "last_seen_at_ms", operation)?,
        revoked_at: decode_optional_time(row, "revoked_at_ms", operation)?,
        matrix_device_revoked_at: decode_optional_time(
            row,
            "matrix_device_revoked_at_ms",
            operation,
        )?,
    };
    if (record.instance.status() == AgentInstanceStatus::Revoked) != record.revoked_at.is_some()
        || (record.matrix_device_revoked_at.is_some() && record.revoked_at.is_none())
    {
        return Err(corrupt_data(operation));
    }
    Ok(record)
}

fn ensure_registration_coherence(registration: &AgentInstanceRegistration) -> RepositoryResult<()> {
    if registration.binding.agent_id() == registration.instance.agent_id()
        && registration.binding.id() == registration.instance.adapter_binding_id()
        && registration.device_id == registration.instance.device_id()
        && registration.binding.state() == AdapterBindingState::Active
        && registration.instance.status() == AgentInstanceStatus::Connecting
    {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "agent_instance.register.contract",
            RepositoryErrorKind::Constraint,
        ))
    }
}

fn ensure_event_contract(instance: &AgentInstance, event: &OutboxMessage) -> RepositoryResult<()> {
    if event.aggregate_type() == "agent_instance"
        && event.aggregate_id() == instance.id().as_uuid()
        && event.event_type() == "agent.instance.registered.v1"
    {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "agent_instance.register.event_contract",
            RepositoryErrorKind::Constraint,
        ))
    }
}

fn ensure_revocation_event_contract(
    instance_id: AgentInstanceId,
    event: &OutboxMessage,
) -> RepositoryResult<()> {
    if event.aggregate_type() == "agent_instance"
        && event.aggregate_id() == instance_id.as_uuid()
        && event.event_type() == "agent.instance.revoked.v1"
    {
        Ok(())
    } else {
        Err(RepositoryError::new(
            "agent_instance.revoke.event_contract",
            RepositoryErrorKind::Constraint,
        ))
    }
}

fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}

fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
