//! 私人房间的邀请链接（`specs/room-people/design.md`）。猜链接和猜 Agent 口令共用一套固定窗口。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        JoinCodeAttemptPolicy, PortFuture, PrivateRoomInviteLinkRecord, PrivateRoomInviteLinkStore,
        SealedInviteLink, SealedSecret, SecretDigest,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use sqlx::postgres::PgRow;

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
};

use super::{
    decode::corrupt_data,
    private_room_agents::{attempt_retry_at, permissions, record_attempt_failure},
};

const INVITE_LINK_COLUMNS: &str = r"
    catalog_entry_id,
    permission_bits,
    created_by_principal_id,
    floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms,
    floor(extract(epoch FROM expires_at) * 1000)::bigint AS expires_at_ms";

impl PrivateRoomInviteLinkStore for PostgresRepositories {
    fn replace_invite_link<'a>(
        &'a self,
        record: &'a PrivateRoomInviteLinkRecord,
        digest: &'a SecretDigest,
        sealed: Option<&'a SealedSecret>,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.invite_link.replace";
            sqlx::query(
                r"INSERT INTO agent_room.private_room_invite_link
                      (catalog_entry_id, secret_digest, sealed_secret, sealed_secret_key_version,
                       permission_bits, created_by_principal_id, created_at, expires_at)
                  VALUES ($1, $2, $3, $4, $5, $6,
                          to_timestamp($7::double precision / 1000.0),
                          to_timestamp($8::double precision / 1000.0))
                  ON CONFLICT (catalog_entry_id) DO UPDATE
                     SET secret_digest = EXCLUDED.secret_digest,
                         sealed_secret = EXCLUDED.sealed_secret,
                         sealed_secret_key_version = EXCLUDED.sealed_secret_key_version,
                         permission_bits = EXCLUDED.permission_bits,
                         created_by_principal_id = EXCLUDED.created_by_principal_id,
                         created_at = EXCLUDED.created_at,
                         expires_at = EXCLUDED.expires_at",
            )
            .bind(record.catalog_id.as_uuid())
            .bind(digest.as_bytes().as_slice())
            .bind(sealed.map(|sealed| sealed.bytes.as_slice()))
            .bind(
                sealed
                    .map(|sealed| i16::try_from(sealed.key_version))
                    .transpose()
                    .map_err(|_| corrupt_data(operation))?,
            )
            .bind(i16::from(record.permissions.bits()))
            .bind(record.created_by.as_uuid())
            .bind(record.created_at.value())
            .bind(record.expires_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn invite_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedInviteLink>>> {
        Box::pin(async move {
            let operation = "private_room.invite_link.find";
            let query = format!(
                "SELECT {INVITE_LINK_COLUMNS}, secret_digest, sealed_secret,
                        sealed_secret_key_version
                   FROM agent_room.private_room_invite_link
                  WHERE catalog_entry_id = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_sealed_invite_link(&row, operation))
                .transpose()
        })
    }

    fn find_invite_link<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomInviteLinkRecord>>> {
        Box::pin(async move {
            let operation = "private_room.invite_link.find_by_digest";
            let query = format!(
                "SELECT {INVITE_LINK_COLUMNS} FROM agent_room.private_room_invite_link
                  WHERE secret_digest = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(digest.as_bytes().as_slice())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_invite_link(&row, operation))
                .transpose()
        })
    }

    fn clear_invite_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "private_room.invite_link.clear";
            let result = sqlx::query(
                "DELETE FROM agent_room.private_room_invite_link WHERE catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn invite_link_retry_at<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<Option<UtcMillis>>> {
        Box::pin(attempt_retry_at(
            self,
            caller,
            now,
            policy,
            "private_room.invite_link.attempts",
        ))
    }

    fn record_invite_link_failure<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(record_attempt_failure(
            self,
            caller,
            now,
            policy,
            "private_room.invite_link.record_failure",
        ))
    }
}

fn decode_invite_link(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomInviteLinkRecord> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let bits: i16 = decode_column(row, "permission_bits", operation)?;
    let created_by: uuid::Uuid = decode_column(row, "created_by_principal_id", operation)?;
    let created_at: i64 = decode_column(row, "created_at_ms", operation)?;
    let expires_at: i64 = decode_column(row, "expires_at_ms", operation)?;
    let time = |value| UtcMillis::new(value).map_err(|error| map_domain_error(operation, &error));
    Ok(PrivateRoomInviteLinkRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        permissions: permissions(bits, operation)?,
        created_by: PrincipalId::from_uuid(created_by),
        created_at: time(created_at)?,
        expires_at: time(expires_at)?,
    })
}

fn decode_sealed_invite_link(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<SealedInviteLink> {
    let record = decode_invite_link(row, operation)?;
    let digest: Vec<u8> = decode_column(row, "secret_digest", operation)?;
    let digest = <[u8; 32]>::try_from(digest.as_slice())
        .map(SecretDigest::from_array)
        .map_err(|_| corrupt_data(operation))?;
    let sealed_secret: Option<Vec<u8>> = decode_column(row, "sealed_secret", operation)?;
    let key_version: Option<i16> = decode_column(row, "sealed_secret_key_version", operation)?;
    let sealed = match (sealed_secret, key_version) {
        (Some(bytes), Some(version)) => Some(SealedSecret {
            key_version: u16::try_from(version).map_err(|_| corrupt_data(operation))?,
            bytes,
        }),
        (None, None) => None,
        _ => return Err(corrupt_data(operation)),
    };
    Ok(SealedInviteLink {
        record,
        digest,
        sealed,
    })
}
