use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{ContentAccessMode, ContentAccessPolicy, MatrixEventId, MatrixRoomId},
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    ids::{ContentId, PrincipalId},
};
use sqlx::postgres::PgRow;

use crate::agents::{decode_column, decode_optional_time, decode_time};

pub(super) const CONTENT_COLUMNS: &str = r"
    content.id AS content_id,
    content.owner_principal_id AS content_owner_principal_id,
    content.storage_key AS content_storage_key,
    content.sha256_digest AS content_sha256_digest,
    content.byte_length AS content_byte_length,
    content.media_type AS content_media_type,
    content.encryption_mode AS content_encryption_mode,
    content.scan_state AS content_scan_state,
    content.lifecycle_state AS content_lifecycle_state,
    floor(extract(epoch FROM content.expires_at) * 1000)::bigint AS content_expires_at_ms,
    floor(extract(epoch FROM content.created_at) * 1000)::bigint AS content_created_at_ms,
    floor(extract(epoch FROM content.deleted_at) * 1000)::bigint AS content_deleted_at_ms";

pub(super) const POLICY_COLUMNS: &str = r"
    policy.content_id AS policy_content_id,
    policy.matrix_room_id AS policy_matrix_room_id,
    policy.matrix_event_id AS policy_matrix_event_id,
    policy.access_mode AS policy_access_mode,
    floor(extract(epoch FROM policy.created_at) * 1000)::bigint AS policy_created_at_ms,
    floor(extract(epoch FROM policy.revoked_at) * 1000)::bigint AS policy_revoked_at_ms";

pub(super) fn decode_content(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<ContentObject> {
    let digest: Vec<u8> = decode_column(row, "content_sha256_digest", operation)?;
    let digest: [u8; 32] = digest.try_into().map_err(|_| corrupt_data(operation))?;
    let byte_length: i64 = decode_column(row, "content_byte_length", operation)?;
    let byte_length = u64::try_from(byte_length).map_err(|_| corrupt_data(operation))?;
    let encryption_mode: String = decode_column(row, "content_encryption_mode", operation)?;
    let scan_state: String = decode_column(row, "content_scan_state", operation)?;
    let lifecycle_state: String = decode_column(row, "content_lifecycle_state", operation)?;

    ContentObject::restore(ContentObjectFields {
        id: ContentId::from_uuid(decode_column(row, "content_id", operation)?),
        owner_principal_id: PrincipalId::from_uuid(decode_column(
            row,
            "content_owner_principal_id",
            operation,
        )?),
        storage_key: ContentStorageKey::new(decode_column::<String>(
            row,
            "content_storage_key",
            operation,
        )?)
        .map_err(|_| corrupt_data(operation))?,
        digest: Sha256Digest::from_bytes(digest),
        byte_length: ContentByteLength::new(byte_length).map_err(|_| corrupt_data(operation))?,
        media_type: ContentMediaType::new(decode_column::<String>(
            row,
            "content_media_type",
            operation,
        )?)
        .map_err(|_| corrupt_data(operation))?,
        encryption_mode: ContentEncryptionMode::try_from(encryption_mode.as_str())
            .map_err(|_| corrupt_data(operation))?,
        scan_state: ContentScanState::try_from(scan_state.as_str())
            .map_err(|_| corrupt_data(operation))?,
        lifecycle_state: ContentLifecycleState::try_from(lifecycle_state.as_str())
            .map_err(|_| corrupt_data(operation))?,
        expires_at: decode_optional_time(row, "content_expires_at_ms", operation)?,
        created_at: decode_time(row, "content_created_at_ms", operation)?,
        deleted_at: decode_optional_time(row, "content_deleted_at_ms", operation)?,
    })
    .map_err(|_| corrupt_data(operation))
}

pub(super) fn decode_policy(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<ContentAccessPolicy> {
    let matrix_event_id: Option<String> = decode_column(row, "policy_matrix_event_id", operation)?;
    let access_mode: String = decode_column(row, "policy_access_mode", operation)?;
    ContentAccessPolicy::restore(
        ContentId::from_uuid(decode_column(row, "policy_content_id", operation)?),
        MatrixRoomId::new(decode_column::<String>(
            row,
            "policy_matrix_room_id",
            operation,
        )?)
        .map_err(|_| corrupt_data(operation))?,
        matrix_event_id
            .map(MatrixEventId::new)
            .transpose()
            .map_err(|_| corrupt_data(operation))?,
        ContentAccessMode::try_from(access_mode.as_str()).map_err(|_| corrupt_data(operation))?,
        decode_time(row, "policy_created_at_ms", operation)?,
        decode_optional_time(row, "policy_revoked_at_ms", operation)?,
    )
    .map_err(|_| corrupt_data(operation))
}

pub(super) const fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
