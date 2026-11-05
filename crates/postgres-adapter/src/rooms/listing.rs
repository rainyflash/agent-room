//! 公开房间的设置（`specs/public-rooms/design.md`）。房间第一次公开时才有一行；每个账户最多 1 间
//! 公开房间，公开时（包括新建时就公开）和转让房主时，在同一个事务里锁住房主再数一遍。

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        PortFuture, PrivateRoomListingRecord, PrivateRoomListingSaveOutcome,
        PrivateRoomListingStore, PrivateRoomSnapshot,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoom,
    room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode},
    time::UtcMillis,
    version::AggregateVersion,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
};

use super::{
    decode::corrupt_data,
    private_room_agents::permissions,
    private_rooms::{create_private_room, ensure_creatable, save_private_room},
};

const SHARE_CODE_INDEX: &str = "private_room_listing_share_code_unique";
const PRIMARY_KEY: &str = "private_room_listing_pkey";

const LISTING_COLUMNS: &str = r"
    catalog_entry_id,
    listed,
    join_mode,
    member_permissions,
    allow_network_agents,
    share_code,
    floor(extract(epoch FROM first_listed_at) * 1000)::bigint AS first_listed_at_ms,
    updated_by,
    floor(extract(epoch FROM updated_at) * 1000)::bigint AS updated_at_ms,
    version";

impl PrivateRoomListingStore for PostgresRepositories {
    fn listing(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomListingRecord>>> {
        Box::pin(async move {
            let operation = "private_room.listing.find";
            let query = format!(
                "SELECT {LISTING_COLUMNS} FROM agent_room.private_room_listing
                  WHERE catalog_entry_id = $1"
            );
            let row = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(catalog_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_listing(&row, operation)).transpose()
        })
    }

    fn listed_share_codes<'a>(
        &'a self,
        catalog_ids: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<Vec<(RoomCatalogId, RoomShareCode)>>> {
        Box::pin(async move {
            let operation = "private_room.listing.share_codes";
            let ids: Vec<uuid::Uuid> = catalog_ids.iter().map(|id| id.as_uuid()).collect();
            let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
                "SELECT catalog_entry_id, share_code FROM agent_room.private_room_listing
                  WHERE catalog_entry_id = ANY($1) AND listed",
            )
            .bind(&ids)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.into_iter()
                .map(|(catalog_id, code)| {
                    RoomShareCode::parse(&code)
                        .map(|code| (RoomCatalogId::from_uuid(catalog_id), code))
                        .map_err(|_| corrupt_data(operation))
                })
                .collect()
        })
    }

    fn listed_rooms_owned_by(
        &self,
        owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>> {
        Box::pin(async move {
            let operation = "private_room.listing.owned_by";
            let rooms: Vec<uuid::Uuid> = sqlx::query_scalar(LISTED_ROOMS_OF_OWNER)
                .bind(owner.as_uuid())
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(rooms.into_iter().map(RoomCatalogId::from_uuid).collect())
        })
    }

    fn save_listing<'a>(
        &'a self,
        record: &'a PrivateRoomListingRecord,
        expected: Option<AggregateVersion>,
        owner: PrincipalId,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async move {
            let operation = "private_room.listing.save";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            if record.settings.listed() {
                // 读到房主以后房间转给了别人：这次公开算的是原房主的名额，让调用方重新读。
                if !still_owned_by(&mut transaction, record.catalog_id, owner, operation).await? {
                    return Ok(PrivateRoomListingSaveOutcome::Stale);
                }
                if owner_lists_another(&mut transaction, owner, record.catalog_id, operation)
                    .await?
                {
                    return Ok(PrivateRoomListingSaveOutcome::OwnerLimitReached);
                }
            }
            let outcome = match expected {
                None => insert_listing(&mut transaction, record, operation).await?,
                Some(expected) => {
                    update_listing(&mut transaction, record, expected, operation).await?
                }
            };
            if outcome == PrivateRoomListingSaveOutcome::Saved {
                transaction
                    .commit()
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
            }
            Ok(outcome)
        })
    }

    fn create_listed_room<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        listing: &'a PrivateRoomListingRecord,
        created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async move {
            let operation = "private_room.listing.create_room";
            ensure_creatable(snapshot, operation)?;
            if listing.catalog_id != snapshot.catalog().id() || !listing.settings.listed() {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Constraint,
                ));
            }
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let owner = snapshot.room().owner_principal_id();
            if owner_lists_another(&mut transaction, owner, listing.catalog_id, operation).await? {
                return Ok(PrivateRoomListingSaveOutcome::OwnerLimitReached);
            }
            // 新建时就公开的房间在 Matrix 上没开加密。
            match create_private_room(&mut transaction, snapshot, created_at, false, operation)
                .await
            {
                Ok(()) => {}
                // 同一个请求重试时房间已经建好了，交给调用方看是不是同一次。
                Err(error) if error.kind() == RepositoryErrorKind::Conflict => {
                    return Ok(PrivateRoomListingSaveOutcome::Stale);
                }
                Err(error) => return Err(error),
            }
            let outcome = insert_listing(&mut transaction, listing, operation).await?;
            if outcome == PrivateRoomListingSaveOutcome::Saved {
                transaction
                    .commit()
                    .await
                    .map_err(|error| map_sqlx_error(operation, &error))?;
            }
            Ok(outcome)
        })
    }

    fn save_transferred_room<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async move {
            let operation = "private_room.listing.save_transferred";
            if expected
                .next()
                .map_err(|error| map_domain_error(operation, &error))?
                != room.version()
            {
                return Err(RepositoryError::new(
                    operation,
                    RepositoryErrorKind::Constraint,
                ));
            }
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            if lock_room_and_read_listed(&mut transaction, room.catalog_id(), operation).await?
                && owner_lists_another(
                    &mut transaction,
                    room.owner_principal_id(),
                    room.catalog_id(),
                    operation,
                )
                .await?
            {
                return Ok(PrivateRoomListingSaveOutcome::OwnerLimitReached);
            }
            save_private_room(&mut transaction, room, expected, changed_at, operation).await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(PrivateRoomListingSaveOutcome::Saved)
        })
    }

    fn mark_room_encrypted(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "private_room.listing.mark_encrypted";
            sqlx::query(
                "UPDATE agent_room.room_catalog_entry
                    SET end_to_end_encrypted = true,
                        updated_at = greatest(updated_at, statement_timestamp())
                  WHERE id = $1 AND kind = 'private_room' AND NOT end_to_end_encrypted",
            )
            .bind(catalog_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }
}

/// 房主名下还在使用中、正公开着的房间。
const LISTED_ROOMS_OF_OWNER: &str = r"
    SELECT listing.catalog_entry_id
      FROM agent_room.private_room_listing AS listing
      JOIN agent_room.room_catalog_entry AS catalog
        ON catalog.id = listing.catalog_entry_id
     WHERE listing.listed
       AND catalog.owner_principal_id = $1
       AND catalog.status = 'active'";

/// 锁住房主那一行，同一个人同时公开两间时一个一个来，再数他名下别的公开房间。
async fn owner_lists_another(
    transaction: &mut Transaction<'_, Postgres>,
    owner: PrincipalId,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    sqlx::query("SELECT 1 FROM agent_room.principal WHERE id = $1 FOR UPDATE")
        .bind(owner.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let listed: Vec<uuid::Uuid> = sqlx::query_scalar(LISTED_ROOMS_OF_OWNER)
        .bind(owner.as_uuid())
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(listed.iter().any(|room| *room != catalog_id.as_uuid()))
}

/// 转让时先锁房间目录那一行，再看这间公开着没有：和 [`still_owned_by`] 抢同一行，同时公开这一间的
/// 请求要么先存完、这里读得到它公开了，要么等转让存完、发现房主已经换了。
async fn lock_room_and_read_listed(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    sqlx::query("SELECT 1 FROM agent_room.room_catalog_entry WHERE id = $1 FOR NO KEY UPDATE")
        .bind(catalog_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let listed: Option<bool> = sqlx::query_scalar(
        "SELECT listed FROM agent_room.private_room_listing WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(listed == Some(true))
}

/// 公开之前看房主还是不是这个人。拿着房间目录那一行直到事务结束，转让要等这次存完。
async fn still_owned_by(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    operation: &'static str,
) -> RepositoryResult<bool> {
    let current: Option<Option<uuid::Uuid>> = sqlx::query_scalar(
        "SELECT owner_principal_id FROM agent_room.room_catalog_entry WHERE id = $1 FOR SHARE",
    )
    .bind(catalog_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(current.flatten() == Some(owner.as_uuid()))
}

async fn insert_listing(
    transaction: &mut Transaction<'_, Postgres>,
    record: &PrivateRoomListingRecord,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomListingSaveOutcome> {
    let result = sqlx::query(
        r"INSERT INTO agent_room.private_room_listing
              (catalog_entry_id, listed, join_mode, member_permissions, allow_network_agents,
               share_code, first_listed_at, updated_by, updated_at, version)
          VALUES ($1, $2, $3, $4, $5, $6,
                  to_timestamp($7::double precision / 1000.0), $8,
                  to_timestamp($9::double precision / 1000.0), $10)",
    )
    .bind(record.catalog_id.as_uuid())
    .bind(record.settings.listed())
    .bind(record.settings.join_mode().as_str())
    .bind(i16::from(record.settings.member_permissions().bits()))
    .bind(record.settings.allow_network_agents())
    .bind(record.share_code.as_str())
    .bind(record.first_listed_at.value())
    .bind(record.updated_by.as_uuid())
    .bind(record.updated_at.value())
    .bind(record.version.value())
    .execute(&mut **transaction)
    .await;
    match result {
        Ok(_) => Ok(PrivateRoomListingSaveOutcome::Saved),
        Err(error) => unique_outcome(&error, operation),
    }
}

async fn update_listing(
    transaction: &mut Transaction<'_, Postgres>,
    record: &PrivateRoomListingRecord,
    expected: AggregateVersion,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomListingSaveOutcome> {
    let result = sqlx::query(
        r"UPDATE agent_room.private_room_listing
             SET listed = $2,
                 join_mode = $3,
                 member_permissions = $4,
                 allow_network_agents = $5,
                 share_code = $6,
                 updated_by = $7,
                 updated_at = to_timestamp($8::double precision / 1000.0),
                 version = $9
           WHERE catalog_entry_id = $1 AND version = $10",
    )
    .bind(record.catalog_id.as_uuid())
    .bind(record.settings.listed())
    .bind(record.settings.join_mode().as_str())
    .bind(i16::from(record.settings.member_permissions().bits()))
    .bind(record.settings.allow_network_agents())
    .bind(record.share_code.as_str())
    .bind(record.updated_by.as_uuid())
    .bind(record.updated_at.value())
    .bind(record.version.value())
    .bind(expected.value())
    .execute(&mut **transaction)
    .await;
    match result {
        Ok(done) if done.rows_affected() == 1 => Ok(PrivateRoomListingSaveOutcome::Saved),
        Ok(_) => Ok(PrivateRoomListingSaveOutcome::Stale),
        Err(error) => unique_outcome(&error, operation),
    }
}

/// 分享码被别的房间占了让调用方换一个；这间房已经有记录（别人刚公开过）算版本对不上。
fn unique_outcome(
    error: &sqlx::Error,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomListingSaveOutcome> {
    match error {
        sqlx::Error::Database(database) if database.constraint() == Some(SHARE_CODE_INDEX) => {
            Ok(PrivateRoomListingSaveOutcome::ShareCodeTaken)
        }
        sqlx::Error::Database(database) if database.constraint() == Some(PRIMARY_KEY) => {
            Ok(PrivateRoomListingSaveOutcome::Stale)
        }
        error => Err(map_sqlx_error(operation, error)),
    }
}

fn decode_listing(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<PrivateRoomListingRecord> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let listed: bool = decode_column(row, "listed", operation)?;
    let join_mode: String = decode_column(row, "join_mode", operation)?;
    let bits: i16 = decode_column(row, "member_permissions", operation)?;
    let allow_network_agents: bool = decode_column(row, "allow_network_agents", operation)?;
    let share_code: String = decode_column(row, "share_code", operation)?;
    let first_listed_at: i64 = decode_column(row, "first_listed_at_ms", operation)?;
    let updated_by: uuid::Uuid = decode_column(row, "updated_by", operation)?;
    let updated_at: i64 = decode_column(row, "updated_at_ms", operation)?;
    let version: i64 = decode_column(row, "version", operation)?;
    let time = |value| UtcMillis::new(value).map_err(|error| map_domain_error(operation, &error));
    let settings = RoomListingSettings::new(
        listed,
        RoomJoinMode::try_from(join_mode.as_str()).map_err(|_| corrupt_data(operation))?,
        permissions(bits, operation)?,
        allow_network_agents,
    )
    .map_err(|_| corrupt_data(operation))?;
    Ok(PrivateRoomListingRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        settings,
        share_code: RoomShareCode::parse(&share_code).map_err(|_| corrupt_data(operation))?,
        first_listed_at: time(first_listed_at)?,
        updated_by: PrincipalId::from_uuid(updated_by),
        updated_at: time(updated_at)?,
        version: AggregateVersion::new(version).map_err(|_| corrupt_data(operation))?,
    })
}
