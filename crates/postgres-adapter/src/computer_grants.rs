//! 这台电脑的授权（`specs/desktop-sign-in/design.md`）：桌面端登录着要一份一次性的授权，
//! Bridge 拿它注册这台电脑。只存摘要，10 分钟内有效，注册时在同一个事务里删掉。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ComputerGrantRecord, ComputerGrantStore, DeviceSessionRegistration, PortFuture,
        SecretDigest, StoredDeviceSession,
    },
};
use agent_room_domain::{devices::Device, ids::PrincipalId, time::UtcMillis};
use sqlx::{Row, postgres::PgRow};

use crate::{
    PostgresRepositories, authentication::lock_account_by_id, devices::store_device_session,
    error::map_sqlx_error,
};

impl ComputerGrantStore for PostgresRepositories {
    fn insert_grant<'a>(
        &'a self,
        grant: &'a ComputerGrantRecord,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "device.computer_grant.insert";
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            sqlx::query(
                r"DELETE FROM agent_room.desktop_computer_grant
                   WHERE expires_at <= to_timestamp($1::double precision / 1000.0)",
            )
            .bind(grant.created_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            sqlx::query(
                r"INSERT INTO agent_room.desktop_computer_grant (
                    grant_digest, principal_id, authenticated_at, created_at, expires_at
                ) VALUES (
                    $1, $2,
                    to_timestamp($3::double precision / 1000.0),
                    to_timestamp($4::double precision / 1000.0),
                    to_timestamp($5::double precision / 1000.0)
                )",
            )
            .bind(grant.grant_digest.as_bytes().as_slice())
            .bind(grant.principal_id.as_uuid())
            .bind(grant.authenticated_at.value())
            .bind(grant.created_at.value())
            .bind(grant.expires_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))
        })
    }

    fn find_grant<'a>(
        &'a self,
        grant_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<ComputerGrantRecord>>> {
        Box::pin(async move {
            let operation = "device.computer_grant.find";
            let row = sqlx::query(
                r"SELECT grant_digest, principal_id,
                    floor(extract(epoch FROM authenticated_at) * 1000)::bigint
                        AS authenticated_at_ms,
                    floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at_ms,
                    floor(extract(epoch FROM expires_at) * 1000)::bigint AS expires_at_ms
                   FROM agent_room.desktop_computer_grant
                   WHERE grant_digest = $1
                     AND expires_at > to_timestamp($2::double precision / 1000.0)",
            )
            .bind(grant_digest.as_bytes().as_slice())
            .bind(now.value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_grant(&row, operation)).transpose()
        })
    }

    fn register_with_grant<'a>(
        &'a self,
        grant: &'a ComputerGrantRecord,
        device: &'a Device,
        session: &'a DeviceSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredDeviceSession>> {
        Box::pin(async move {
            let operation = "device.register_with_grant";
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            // 删得掉才算用上：同一份授权并发注册两次，只有先删到的那次往下走。
            let consumed = sqlx::query(
                r"DELETE FROM agent_room.desktop_computer_grant
                   WHERE grant_digest = $1
                     AND principal_id = $2
                     AND expires_at > to_timestamp($3::double precision / 1000.0)",
            )
            .bind(grant.grant_digest.as_bytes().as_slice())
            .bind(grant.principal_id.as_uuid())
            .bind(session.issued_at.value())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .rows_affected();
            if consumed != 1 {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::NotFound,
                ));
            }
            let account =
                lock_account_by_id(&mut transaction, grant.principal_id, operation).await?;
            let stored =
                store_device_session(&mut transaction, account, device, session, operation).await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(stored)
        })
    }
}

fn decode_grant(row: &PgRow, operation: &'static str) -> RepositoryResult<ComputerGrantRecord> {
    let digest: Vec<u8> = row
        .try_get("grant_digest")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let grant_digest =
        SecretDigest::from_array(digest.try_into().map_err(|_| corrupt_data(operation))?);
    let principal_id: uuid::Uuid = row
        .try_get("principal_id")
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(ComputerGrantRecord {
        grant_digest,
        principal_id: PrincipalId::from_uuid(principal_id),
        authenticated_at: decode_time(row, "authenticated_at_ms", operation)?,
        created_at: decode_time(row, "created_at_ms", operation)?,
        expires_at: decode_time(row, "expires_at_ms", operation)?,
    })
}

fn decode_time(row: &PgRow, column: &str, operation: &'static str) -> RepositoryResult<UtcMillis> {
    let value: i64 = row
        .try_get(column)
        .map_err(|error| map_sqlx_error(operation, &error))?;
    UtcMillis::new(value).map_err(|_| corrupt_data(operation))
}

fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}
