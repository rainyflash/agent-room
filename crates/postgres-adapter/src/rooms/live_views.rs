//! 分享页放实况（`specs/redesign/project-rooms.md`“分享页放实况”，表 `room_live_view`，3P-6a）。
//!
//! - 打开时在同一句里再看一遍那几样（公开着、在使用中、不加密、直接进、没被平台拿掉、不是子房间），锁着
//!   房间目录和公开设置那两行（`FOR SHARE`）：和改设置（`listing.rs` 的 `save_listing`）、归档
//!   （`private_rooms.rs` 的 `write_room_rows`）撞上时，要么它们等这一句存完、再把这一行删掉，要么这一句
//!   等它们存完、读到改过的设置不再记下。
//! - 关了就删：改成要申请或只能邀请、改回私密、目录记成加密（`mark_room_encrypted`）、归档或关掉、删账户时
//!   关掉他的房间，都在改房间的那个事务里、改完那几行以后删（[`turn_off_live_view`]，`accounts.rs` 的
//!   `archive_private_rooms` 另有一句按房主删）。平台冻结、拿掉时不删。
//! - 项目房间建好就开着（[`enable_for_project_room`]，建房的那个事务里）。
//! - 分享页按分享码找放着实况的房间（[`LiveRoomLookup`]，3P-6a-2）：和打开时查的是同几样，再加上
//!   房间实例在用、开着实况。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        EnableRoomLiveViewOutcome, LiveRoomLookup, LiveRoomRecord, MatrixRoomId, PortFuture,
        RoomLiveViewFacts, RoomLiveViewRecord, RoomLiveViewStore,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_column, decode_time},
    error::map_sqlx_error,
};

use super::decode::corrupt_data;

impl RoomLiveViewStore for PostgresRepositories {
    fn live_view_facts(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomLiveViewFacts>>> {
        Box::pin(async move {
            let operation = "room_live.facts";
            let row = sqlx::query(
                r"SELECT catalog.end_to_end_encrypted,
                         listing.listed,
                         listing.join_mode,
                         EXISTS (
                             SELECT 1 FROM agent_room.public_room_platform_hold AS hold
                              WHERE hold.catalog_entry_id = catalog.id
                         ) AS held,
                         live.enabled_by,
                         floor(extract(epoch FROM live.enabled_at) * 1000)::bigint
                             AS enabled_at_ms
                    FROM agent_room.room_catalog_entry AS catalog
                    LEFT JOIN agent_room.private_room_listing AS listing
                      ON listing.catalog_entry_id = catalog.id
                    LEFT JOIN agent_room.room_live_view AS live
                      ON live.catalog_entry_id = catalog.id
                   WHERE catalog.id = $1 AND catalog.kind = 'private_room'",
            )
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_facts(&row, operation)).transpose()
        })
    }

    fn enable_live_view(
        &self,
        catalog_id: RoomCatalogId,
        enabled_by: PrincipalId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<EnableRoomLiveViewOutcome>> {
        Box::pin(async move {
            let operation = "room_live.enable";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let inserted: Option<i64> = sqlx::query_scalar(
                r"WITH eligible AS (
                      SELECT catalog.id
                        FROM agent_room.room_catalog_entry AS catalog
                        JOIN agent_room.private_room_listing AS listing
                          ON listing.catalog_entry_id = catalog.id
                       WHERE catalog.id = $1
                         AND catalog.kind = 'private_room'
                         AND catalog.status = 'active'
                         AND NOT catalog.end_to_end_encrypted
                         AND listing.listed
                         AND listing.join_mode = 'open'
                         AND NOT EXISTS (
                             SELECT 1 FROM agent_room.public_room_platform_hold AS hold
                              WHERE hold.catalog_entry_id = catalog.id
                         )
                         AND NOT EXISTS (
                             SELECT 1 FROM agent_room.private_room_parent AS link
                              WHERE link.catalog_entry_id = catalog.id
                         )
                         FOR SHARE OF catalog, listing
                  )
                  INSERT INTO agent_room.room_live_view (catalog_entry_id, enabled_by, enabled_at)
                  SELECT id, $2, to_timestamp($3::double precision / 1000.0) FROM eligible
                  ON CONFLICT (catalog_entry_id) DO NOTHING
                  RETURNING floor(extract(epoch FROM enabled_at) * 1000)::bigint",
            )
            .bind(catalog_id.as_uuid())
            .bind(enabled_by.as_uuid())
            .bind(at.value())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            let outcome = if let Some(enabled_at) = inserted {
                EnableRoomLiveViewOutcome::Enabled(RoomLiveViewRecord {
                    enabled_by,
                    enabled_at: UtcMillis::new(enabled_at).map_err(|_| corrupt_data(operation))?,
                })
            } else {
                // 没记下：要么本来就开着，要么这一下不满足了。
                existing(&mut transaction, catalog_id, operation)
                    .await?
                    .map_or(
                        EnableRoomLiveViewOutcome::NotEligible,
                        EnableRoomLiveViewOutcome::AlreadyEnabled,
                    )
            };
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(outcome)
        })
    }

    fn disable_live_view(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "room_live.disable";
            sqlx::query("DELETE FROM agent_room.room_live_view WHERE catalog_entry_id = $1")
                .bind(catalog_id.as_uuid())
                .execute(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }
}

impl LiveRoomLookup for PostgresRepositories {
    fn live_room<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, RepositoryResult<Option<LiveRoomRecord>>> {
        Box::pin(async move {
            let operation = "room_live.lookup";
            let row: Option<(uuid::Uuid, String, i64)> = sqlx::query_as(
                r"SELECT catalog.id,
                         instance.matrix_room_id,
                         floor(extract(epoch FROM live.enabled_at) * 1000)::bigint
                    FROM agent_room.private_room_listing AS listing
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = listing.catalog_entry_id
                    JOIN agent_room.room_live_view AS live
                      ON live.catalog_entry_id = catalog.id
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = catalog.id
                   WHERE listing.share_code = $1
                     AND listing.listed
                     AND listing.join_mode = 'open'
                     AND catalog.kind = 'private_room'
                     AND catalog.status = 'active'
                     AND NOT catalog.end_to_end_encrypted
                     AND instance.state = 'active'
                     AND NOT EXISTS (
                         SELECT 1 FROM agent_room.public_room_platform_hold AS hold
                          WHERE hold.catalog_entry_id = catalog.id
                     )
                     AND NOT EXISTS (
                         SELECT 1 FROM agent_room.private_room_parent AS link
                          WHERE link.catalog_entry_id = catalog.id
                     )",
            )
            .bind(share_code.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|(catalog_id, matrix_room, enabled_at)| {
                Ok(LiveRoomRecord {
                    catalog_id: RoomCatalogId::from_uuid(catalog_id),
                    matrix_room_id: MatrixRoomId::new(matrix_room)
                        .map_err(|_| corrupt_data(operation))?,
                    enabled_at: UtcMillis::new(enabled_at).map_err(|_| corrupt_data(operation))?,
                })
            })
            .transpose()
        })
    }
}

/// 关掉这间房的实况，有就删。要和改动房间的那次存放在同一个事务里，放在改房间目录、公开设置那几句
/// 之后：同时在开的那一句要么已经存完、这里删得到，要么等这边存完、读到改过的设置。
pub(super) async fn turn_off_live_view(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query("DELETE FROM agent_room.room_live_view WHERE catalog_entry_id = $1")
        .bind(catalog_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 项目房间建好就开着实况（设计“分享页放实况”的默认）：记的是认领的人、建房的那一刻。
pub(super) async fn enable_for_project_room(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    enabled_by: PrincipalId,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.room_live_view (catalog_entry_id, enabled_by, enabled_at)
          VALUES ($1, $2, to_timestamp($3::double precision / 1000.0))
          ON CONFLICT (catalog_entry_id) DO NOTHING",
    )
    .bind(catalog_id.as_uuid())
    .bind(enabled_by.as_uuid())
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn existing(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<Option<RoomLiveViewRecord>> {
    let row = sqlx::query(
        r"SELECT enabled_by,
                 floor(extract(epoch FROM enabled_at) * 1000)::bigint AS enabled_at_ms
            FROM agent_room.room_live_view
           WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    row.map(|row| {
        let enabled_by: uuid::Uuid = decode_column(&row, "enabled_by", operation)?;
        Ok(RoomLiveViewRecord {
            enabled_by: PrincipalId::from_uuid(enabled_by),
            enabled_at: decode_time(&row, "enabled_at_ms", operation)?,
        })
    })
    .transpose()
}

fn decode_facts(row: &PgRow, operation: &'static str) -> RepositoryResult<RoomLiveViewFacts> {
    let listed: Option<bool> = decode_column(row, "listed", operation)?;
    let join_mode: Option<String> = decode_column(row, "join_mode", operation)?;
    let listing = match (listed, join_mode) {
        (Some(listed), Some(join_mode)) => Some((
            listed,
            RoomJoinMode::try_from(join_mode.as_str()).map_err(|_| corrupt_data(operation))?,
        )),
        (None, None) => None,
        _ => return Err(corrupt_data(operation)),
    };
    let enabled_by: Option<uuid::Uuid> = decode_column(row, "enabled_by", operation)?;
    let enabled_at: Option<i64> = decode_column(row, "enabled_at_ms", operation)?;
    let live_view = match (enabled_by, enabled_at) {
        (Some(enabled_by), Some(enabled_at)) => Some(RoomLiveViewRecord {
            enabled_by: PrincipalId::from_uuid(enabled_by),
            enabled_at: UtcMillis::new(enabled_at).map_err(|_| corrupt_data(operation))?,
        }),
        (None, None) => None,
        _ => return Err(corrupt_data(operation)),
    };
    Ok(RoomLiveViewFacts {
        listing,
        end_to_end_encrypted: decode_column(row, "end_to_end_encrypted", operation)?,
        platform_hold: decode_column(row, "held", operation)?,
        live_view,
    })
}
