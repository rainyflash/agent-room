use agent_room_bridge_core::handoffs::{HandoffStoreFailure, HandoffStoreFailureKind};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffFailureCode,
        HandoffPermission, HandoffPermissions, HandoffPurpose, HandoffSource, HandoffSourceActor,
        HandoffSourceEventId, HandoffStatus,
    },
    ids::{HandoffId, PrincipalId},
    messages::{MessageProvenance, MessageRiskFlag, MessageRiskFlags},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use uuid::{Uuid, Version};

use crate::error::{SqliteFailureKind, classify};

const SELECT_HANDOFF: &str = "SELECT handoff_id, requester_agent_id, requester_instance_id,
            source_room_id, source_event_id, source_message_id, source_agent_id,
            source_instance_id, source_provenance, target_agent_id, target_instance_id,
            content_id, content_digest, content_byte_length, content_media_type,
            permissions_json, purpose, risk_flags_json, proposed_at_unix_ms,
            expires_at_unix_ms, status, approved_by_principal_id,
            approved_at_unix_ms, delivered_at_unix_ms, consumed_at_unix_ms,
            resolved_at_unix_ms, failure_code, version
     FROM context_handoff WHERE handoff_id = ?";

#[derive(Debug)]
pub(super) struct LoadedHandoff {
    pub handoff: ContextHandoff,
    pub version: i64,
}

#[derive(Debug, FromRow)]
struct HandoffRow {
    handoff_id: String,
    requester_agent_id: String,
    requester_instance_id: String,
    source_room_id: String,
    source_event_id: String,
    source_message_id: String,
    source_agent_id: String,
    source_instance_id: String,
    source_provenance: String,
    target_agent_id: String,
    target_instance_id: String,
    content_id: String,
    content_digest: Vec<u8>,
    content_byte_length: i64,
    content_media_type: String,
    permissions_json: String,
    purpose: String,
    risk_flags_json: String,
    proposed_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    status: String,
    approved_by_principal_id: Option<String>,
    approved_at_unix_ms: Option<i64>,
    delivered_at_unix_ms: Option<i64>,
    consumed_at_unix_ms: Option<i64>,
    resolved_at_unix_ms: Option<i64>,
    failure_code: Option<String>,
    version: i64,
}

#[derive(Debug, FromRow)]
pub(super) struct StoredPackage {
    pub key_version: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub(super) async fn load_from_pool(
    pool: &SqlitePool,
    handoff_id: HandoffId,
) -> Result<Option<LoadedHandoff>, HandoffStoreFailure> {
    sqlx::query_as::<_, HandoffRow>(SELECT_HANDOFF)
        .bind(handoff_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|error| map_sqlx_error(&error))?
        .map(|row| decode_handoff(&row))
        .transpose()
}

pub(super) async fn load_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    handoff_id: HandoffId,
) -> Result<Option<LoadedHandoff>, HandoffStoreFailure> {
    sqlx::query_as::<_, HandoffRow>(SELECT_HANDOFF)
        .bind(handoff_id.to_string())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?
        .map(|row| decode_handoff(&row))
        .transpose()
}

pub(super) async fn insert_handoff(
    transaction: &mut Transaction<'_, Sqlite>,
    handoff: &ContextHandoff,
) -> Result<bool, HandoffStoreFailure> {
    let fields = handoff.fields();
    let permissions = fields
        .permissions
        .iter()
        .map(HandoffPermission::as_str)
        .collect::<Vec<_>>();
    let permissions_json =
        serde_json::to_string(&permissions).map_err(|_| unavailable_failure())?;
    let risk_flags = fields
        .risk_flags
        .iter()
        .map(MessageRiskFlag::as_str)
        .collect::<Vec<_>>();
    let risk_flags_json = serde_json::to_string(&risk_flags).map_err(|_| unavailable_failure())?;
    let content_byte_length =
        i64::try_from(fields.content.byte_length().value()).map_err(|_| corrupt_failure())?;
    let result = sqlx::query(
        "INSERT INTO context_handoff (
            handoff_id, requester_agent_id, requester_instance_id,
            source_room_id, source_event_id, source_message_id,
            source_agent_id, source_instance_id, source_provenance,
            target_agent_id, target_instance_id, content_id, content_digest,
            content_byte_length, content_media_type, permissions_json, purpose,
            risk_flags_json, proposed_at_unix_ms, expires_at_unix_ms, status,
            approved_by_principal_id, approved_at_unix_ms, delivered_at_unix_ms,
            consumed_at_unix_ms, resolved_at_unix_ms, failure_code, version
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT(handoff_id) DO NOTHING",
    )
    .bind(fields.id.to_string())
    .bind(fields.requester_agent_id.to_string())
    .bind(fields.requester_instance_id.to_string())
    .bind(fields.source.room_id().as_str())
    .bind(fields.source.event_id().as_str())
    .bind(fields.source.message_id().to_string())
    .bind(fields.source.actor().agent_id().to_string())
    .bind(fields.source.actor().instance_id().to_string())
    .bind(fields.source.actor().provenance().as_str())
    .bind(fields.target_agent_id.to_string())
    .bind(fields.target_instance_id.to_string())
    .bind(fields.content.content_id().to_string())
    .bind(fields.content.digest().as_bytes().to_vec())
    .bind(content_byte_length)
    .bind(fields.content.media_type().as_str())
    .bind(permissions_json)
    .bind(fields.purpose.as_str())
    .bind(risk_flags_json)
    .bind(fields.proposed_at.value())
    .bind(fields.expires_at.value())
    .bind(handoff.status().as_str())
    .bind(handoff.approved_by_principal_id().map(|id| id.to_string()))
    .bind(handoff.approved_at().map(UtcMillis::value))
    .bind(handoff.delivered_at().map(UtcMillis::value))
    .bind(handoff.consumed_at().map(UtcMillis::value))
    .bind(handoff.resolved_at().map(UtcMillis::value))
    .bind(handoff.failure_code().map(HandoffFailureCode::as_str))
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn update_handoff_state(
    transaction: &mut Transaction<'_, Sqlite>,
    loaded: &LoadedHandoff,
) -> Result<(), HandoffStoreFailure> {
    let handoff = &loaded.handoff;
    let result = sqlx::query(
        "UPDATE context_handoff
         SET status = ?, approved_by_principal_id = ?, approved_at_unix_ms = ?,
             delivered_at_unix_ms = ?, consumed_at_unix_ms = ?, resolved_at_unix_ms = ?,
             failure_code = ?, version = version + 1
         WHERE handoff_id = ? AND version = ?",
    )
    .bind(handoff.status().as_str())
    .bind(handoff.approved_by_principal_id().map(|id| id.to_string()))
    .bind(handoff.approved_at().map(UtcMillis::value))
    .bind(handoff.delivered_at().map(UtcMillis::value))
    .bind(handoff.consumed_at().map(UtcMillis::value))
    .bind(handoff.resolved_at().map(UtcMillis::value))
    .bind(handoff.failure_code().map(HandoffFailureCode::as_str))
    .bind(handoff.fields().id.to_string())
    .bind(loaded.version)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(conflict_failure())
    }
}

pub(super) async fn insert_package(
    transaction: &mut Transaction<'_, Sqlite>,
    handoff_id: HandoffId,
    package: &super::crypto::EncryptedPackage,
) -> Result<(), HandoffStoreFailure> {
    sqlx::query(
        "INSERT INTO context_handoff_package
         (handoff_id, key_version, nonce, ciphertext) VALUES (?, ?, ?, ?)",
    )
    .bind(handoff_id.to_string())
    .bind(package.key_version)
    .bind(&package.nonce)
    .bind(&package.ciphertext)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    Ok(())
}

pub(super) async fn load_package(
    transaction: &mut Transaction<'_, Sqlite>,
    handoff_id: HandoffId,
) -> Result<Option<StoredPackage>, HandoffStoreFailure> {
    sqlx::query_as::<_, StoredPackage>(
        "SELECT key_version, nonce, ciphertext
         FROM context_handoff_package WHERE handoff_id = ?",
    )
    .bind(handoff_id.to_string())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))
}

pub(super) async fn delete_package(
    transaction: &mut Transaction<'_, Sqlite>,
    handoff_id: HandoffId,
) -> Result<(), HandoffStoreFailure> {
    sqlx::query("DELETE FROM context_handoff_package WHERE handoff_id = ?")
        .bind(handoff_id.to_string())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
    Ok(())
}

pub(super) fn same_intent(left: &ContextHandoff, right: &ContextHandoff) -> bool {
    left.fields() == right.fields()
        && left.approved_by_principal_id() == right.approved_by_principal_id()
        && left.approved_at() == right.approved_at()
}

pub(super) fn is_terminal(status: HandoffStatus) -> bool {
    matches!(
        status,
        HandoffStatus::Consumed
            | HandoffStatus::Declined
            | HandoffStatus::Revoked
            | HandoffStatus::Expired
            | HandoffStatus::Failed
    )
}

pub(super) fn map_sqlx_error(error: &sqlx::Error) -> HandoffStoreFailure {
    let kind = match classify(error) {
        SqliteFailureKind::NotFound => HandoffStoreFailureKind::NotFound,
        SqliteFailureKind::Conflict => HandoffStoreFailureKind::Conflict,
        SqliteFailureKind::Corrupt => HandoffStoreFailureKind::Corrupt,
        SqliteFailureKind::Unavailable => HandoffStoreFailureKind::Unavailable,
    };
    HandoffStoreFailure::new(kind)
}

fn decode_handoff(row: &HandoffRow) -> Result<LoadedHandoff, HandoffStoreFailure> {
    if row.version <= 0 {
        return Err(corrupt_failure());
    }
    let status = decode_status(&row.status)?;
    let approved_by_principal_id = row
        .approved_by_principal_id
        .as_deref()
        .map(decode_id::<PrincipalId>)
        .transpose()?;
    let approved_at = decode_optional_time(row.approved_at_unix_ms)?;
    let delivered_at = decode_optional_time(row.delivered_at_unix_ms)?;
    let consumed_at = decode_optional_time(row.consumed_at_unix_ms)?;
    let resolved_at = decode_optional_time(row.resolved_at_unix_ms)?;
    let failure_code = row
        .failure_code
        .as_deref()
        .map(HandoffFailureCode::new)
        .transpose()
        .map_err(|_| corrupt_failure())?;
    let mut handoff =
        ContextHandoff::propose(decode_fields(row)?).map_err(|_| corrupt_failure())?;

    match (approved_by_principal_id, approved_at) {
        (Some(principal_id), Some(occurred_at)) => handoff
            .approve(principal_id, occurred_at)
            .map_err(|_| corrupt_failure())?,
        (None, None) => {}
        _ => return Err(corrupt_failure()),
    }
    if let Some(occurred_at) = delivered_at {
        handoff
            .mark_delivered(occurred_at)
            .map_err(|_| corrupt_failure())?;
    }
    restore_terminal_state(
        &mut handoff,
        status,
        consumed_at,
        resolved_at,
        failure_code.clone(),
    )?;
    if handoff.status() != status
        || handoff.approved_by_principal_id() != approved_by_principal_id
        || handoff.approved_at() != approved_at
        || handoff.delivered_at() != delivered_at
        || handoff.consumed_at() != consumed_at
        || handoff.resolved_at() != resolved_at
        || handoff.failure_code() != failure_code.as_ref()
    {
        return Err(corrupt_failure());
    }
    Ok(LoadedHandoff {
        handoff,
        version: row.version,
    })
}

fn decode_fields(row: &HandoffRow) -> Result<ContextHandoffFields, HandoffStoreFailure> {
    let digest =
        <[u8; 32]>::try_from(row.content_digest.as_slice()).map_err(|_| corrupt_failure())?;
    let content_byte_length =
        u64::try_from(row.content_byte_length).map_err(|_| corrupt_failure())?;
    let source_actor = HandoffSourceActor::new(
        decode_id(&row.source_agent_id)?,
        decode_id(&row.source_instance_id)?,
        MessageProvenance::try_from(row.source_provenance.as_str())
            .map_err(|_| corrupt_failure())?,
    );
    Ok(ContextHandoffFields {
        id: decode_id(&row.handoff_id)?,
        requester_agent_id: decode_id(&row.requester_agent_id)?,
        requester_instance_id: decode_id(&row.requester_instance_id)?,
        source: HandoffSource::new(
            MatrixRoomReference::new(row.source_room_id.clone()).map_err(|_| corrupt_failure())?,
            HandoffSourceEventId::new(row.source_event_id.clone())
                .map_err(|_| corrupt_failure())?,
            decode_id(&row.source_message_id)?,
            source_actor,
        ),
        target_agent_id: decode_id(&row.target_agent_id)?,
        target_instance_id: decode_id(&row.target_instance_id)?,
        content: HandoffContentReference::new(
            decode_id(&row.content_id)?,
            Sha256Digest::from_bytes(digest),
            ContentByteLength::new(content_byte_length).map_err(|_| corrupt_failure())?,
            ContentMediaType::new(row.content_media_type.clone()).map_err(|_| corrupt_failure())?,
        ),
        permissions: decode_permissions(&row.permissions_json)?,
        purpose: HandoffPurpose::try_from(row.purpose.as_str()).map_err(|_| corrupt_failure())?,
        risk_flags: decode_risk_flags(&row.risk_flags_json)?,
        proposed_at: decode_time(row.proposed_at_unix_ms)?,
        expires_at: decode_time(row.expires_at_unix_ms)?,
    })
}

fn restore_terminal_state(
    handoff: &mut ContextHandoff,
    status: HandoffStatus,
    consumed_at: Option<UtcMillis>,
    resolved_at: Option<UtcMillis>,
    failure_code: Option<HandoffFailureCode>,
) -> Result<(), HandoffStoreFailure> {
    match status {
        HandoffStatus::Proposed | HandoffStatus::Approved | HandoffStatus::Delivered => Ok(()),
        HandoffStatus::Consumed => handoff
            .consume(consumed_at.ok_or_else(corrupt_failure)?)
            .map_err(|_| corrupt_failure()),
        HandoffStatus::Declined => handoff
            .decline(resolved_at.ok_or_else(corrupt_failure)?)
            .map_err(|_| corrupt_failure()),
        HandoffStatus::Revoked => handoff
            .revoke(resolved_at.ok_or_else(corrupt_failure)?)
            .map_err(|_| corrupt_failure()),
        HandoffStatus::Expired => handoff
            .expire(resolved_at.ok_or_else(corrupt_failure)?)
            .map_err(|_| corrupt_failure()),
        HandoffStatus::Failed => handoff
            .fail(
                failure_code.ok_or_else(corrupt_failure)?,
                resolved_at.ok_or_else(corrupt_failure)?,
            )
            .map_err(|_| corrupt_failure()),
    }
}

fn decode_permissions(value: &str) -> Result<HandoffPermissions, HandoffStoreFailure> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|_| corrupt_failure())?;
    let permissions = values
        .iter()
        .map(|value| HandoffPermission::try_from(value.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt_failure())?;
    let decoded = HandoffPermissions::new(permissions).map_err(|_| corrupt_failure())?;
    if decoded.iter().count() != values.len() {
        return Err(corrupt_failure());
    }
    Ok(decoded)
}

fn decode_risk_flags(value: &str) -> Result<MessageRiskFlags, HandoffStoreFailure> {
    let values = serde_json::from_str::<Vec<String>>(value).map_err(|_| corrupt_failure())?;
    let flags = values
        .iter()
        .map(|value| MessageRiskFlag::new(value.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt_failure())?;
    let decoded = MessageRiskFlags::new(flags).map_err(|_| corrupt_failure())?;
    if decoded.iter().count() != values.len() {
        return Err(corrupt_failure());
    }
    Ok(decoded)
}

fn decode_status(value: &str) -> Result<HandoffStatus, HandoffStoreFailure> {
    match value {
        "proposed" => Ok(HandoffStatus::Proposed),
        "approved" => Ok(HandoffStatus::Approved),
        "delivered" => Ok(HandoffStatus::Delivered),
        "consumed" => Ok(HandoffStatus::Consumed),
        "declined" => Ok(HandoffStatus::Declined),
        "revoked" => Ok(HandoffStatus::Revoked),
        "expired" => Ok(HandoffStatus::Expired),
        "failed" => Ok(HandoffStatus::Failed),
        _ => Err(corrupt_failure()),
    }
}

fn decode_id<T>(value: &str) -> Result<T, HandoffStoreFailure>
where
    T: From<Uuid>,
{
    let value = Uuid::parse_str(value).map_err(|_| corrupt_failure())?;
    if value.get_version() != Some(Version::SortRand) {
        return Err(corrupt_failure());
    }
    Ok(T::from(value))
}

fn decode_time(value: i64) -> Result<UtcMillis, HandoffStoreFailure> {
    UtcMillis::new(value).map_err(|_| corrupt_failure())
}

fn decode_optional_time(value: Option<i64>) -> Result<Option<UtcMillis>, HandoffStoreFailure> {
    value.map(decode_time).transpose()
}

const fn conflict_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict)
}

const fn corrupt_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Corrupt)
}

const fn unavailable_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Unavailable)
}
