//! 控制面替账户保管的密钥存储钥匙（ADR 0011）：每个主体一行，封存后入库。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{AccountEncryptionKeyRepository, PortFuture, SealedSecret, StoredEncryptionKey},
};
use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use crate::{PostgresRepositories, agents::corrupt_data, error::map_sqlx_error};

impl AccountEncryptionKeyRepository for PostgresRepositories {
    fn find_encryption_key(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<StoredEncryptionKey>>> {
        Box::pin(async move {
            let operation = "account.find_encryption_key";
            let row = sqlx::query_as::<_, (String, i16, Vec<u8>)>(
                r"SELECT key_id, key_version, sealed
                    FROM agent_room.principal_encryption_key
                   WHERE principal_id = $1",
            )
            .bind(principal_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|(key_id, version, bytes)| {
                Ok(StoredEncryptionKey {
                    key_id,
                    sealed: SealedSecret {
                        key_version: u16::try_from(version).map_err(|_| corrupt_data(operation))?,
                        bytes,
                    },
                })
            })
            .transpose()
        })
    }

    fn put_encryption_key<'a>(
        &'a self,
        principal_id: PrincipalId,
        key: &'a StoredEncryptionKey,
        updated_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "account.put_encryption_key";
            let version =
                i16::try_from(key.sealed.key_version).map_err(|_| corrupt_data(operation))?;
            sqlx::query(
                r"INSERT INTO agent_room.principal_encryption_key
                      (principal_id, key_id, sealed, key_version, updated_at)
                  VALUES ($1, $2, $3, $4, to_timestamp($5::double precision / 1000.0))
                  ON CONFLICT (principal_id) DO UPDATE
                     SET key_id = EXCLUDED.key_id,
                         sealed = EXCLUDED.sealed,
                         key_version = EXCLUDED.key_version,
                         updated_at = EXCLUDED.updated_at",
            )
            .bind(principal_id.as_uuid())
            .bind(key.key_id.as_str())
            .bind(key.sealed.bytes.as_slice())
            .bind(version)
            .bind(updated_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }
}
