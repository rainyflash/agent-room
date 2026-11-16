//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，表 `room_excerpt`，3P-7a）。
//!
//! - 只存是哪几句（事件 ID）、哪间房、第一句的时间、谁分享的、什么时候，不存话。
//! - 记的时候锁着这间房实况那一行（`FOR SHARE`），还是分享时读到的那一刻开着才记。关实况的几处
//!   （[`super::live_views::turn_off_live_view`]、`disable_live_view`）先删实况那一行、再删这间房的段，分成
//!   两句：和这里撞上时，要么这里等关完、读到那一行没了记不下，要么那边等这里存完、第二句看得到刚记下的
//!   这一段一起删。删账户时关他名下房间的实况和他分享的段在 `accounts.rs` 里。
//! - 每天清一次（[`RoomExcerptStore::prune_excerpts`]）。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        ExcerptRoomRecord, InsertRoomExcerptOutcome, MatrixEventId, MatrixRoomId, PortFuture,
        RoomExcerptListing, RoomExcerptRecord, RoomExcerptStore,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_excerpts::RoomExcerptCode,
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::{decode_column, decode_time},
    error::map_sqlx_error,
};

use super::decode::corrupt_data;

/// 一间房最多列这么多段（最近的）。
const MAX_LISTED: i64 = 200;

const EXCERPT_COLUMNS: &str = r"
    SELECT excerpt.code,
           excerpt.catalog_entry_id,
           excerpt.event_ids,
           floor(extract(epoch FROM excerpt.first_event_at) * 1000)::bigint AS first_event_at_ms,
           excerpt.shared_by,
           floor(extract(epoch FROM excerpt.shared_at) * 1000)::bigint AS shared_at_ms
      FROM agent_room.room_excerpt AS excerpt";

impl RoomExcerptStore for PostgresRepositories {
    fn excerpt_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<ExcerptRoomRecord>>> {
        Box::pin(async move {
            let operation = "room_excerpt.room";
            // 和分享页按分享码找放着实况的房间（`live_views.rs` 的 `LiveRoomLookup`）同几样。
            let row: Option<(String, i64, String)> = sqlx::query_as(
                r"SELECT instance.matrix_room_id,
                         floor(extract(epoch FROM live.enabled_at) * 1000)::bigint,
                         listing.share_code
                    FROM agent_room.private_room_listing AS listing
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = listing.catalog_entry_id
                    JOIN agent_room.room_live_view AS live
                      ON live.catalog_entry_id = catalog.id
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = catalog.id
                   WHERE catalog.id = $1
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
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|(matrix_room, enabled_at, share_code)| {
                Ok(ExcerptRoomRecord {
                    matrix_room_id: MatrixRoomId::new(matrix_room)
                        .map_err(|_| corrupt_data(operation))?,
                    enabled_at: UtcMillis::new(enabled_at).map_err(|_| corrupt_data(operation))?,
                    share_code: RoomShareCode::parse(&share_code)
                        .map_err(|_| corrupt_data(operation))?,
                })
            })
            .transpose()
        })
    }

    fn insert_excerpt<'a>(
        &'a self,
        excerpt: &'a RoomExcerptRecord,
        enabled_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<InsertRoomExcerptOutcome>> {
        Box::pin(async move {
            let operation = "room_excerpt.insert";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let outcome = insert_locked(&mut transaction, excerpt, enabled_at, operation).await?;
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(outcome)
        })
    }

    fn room_excerpts(
        &self,
        catalog_id: RoomCatalogId,
        shared_by: Option<PrincipalId>,
    ) -> PortFuture<'_, RepositoryResult<RoomExcerptListing>> {
        Box::pin(async move {
            let operation = "room_excerpt.list";
            let share_code: Option<String> = sqlx::query_scalar(
                "SELECT share_code FROM agent_room.private_room_listing WHERE catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            // 拼进去的只有上面的常量，参数都经绑定。
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "{EXCERPT_COLUMNS}
                  WHERE excerpt.catalog_entry_id = $1
                    AND ($2::uuid IS NULL OR excerpt.shared_by = $2)
                  ORDER BY excerpt.shared_at DESC, excerpt.code
                  LIMIT $3"
            )))
            .bind(catalog_id.as_uuid())
            .bind(shared_by.map(PrincipalId::as_uuid))
            .bind(MAX_LISTED)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(RoomExcerptListing {
                share_code: share_code
                    .map(|code| RoomShareCode::parse(&code).map_err(|_| corrupt_data(operation)))
                    .transpose()?,
                excerpts: rows
                    .iter()
                    .map(|row| decode_excerpt(row, operation))
                    .collect::<RepositoryResult<_>>()?,
            })
        })
    }

    fn find_excerpt<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomExcerptRecord>>> {
        Box::pin(async move {
            let operation = "room_excerpt.find";
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "{EXCERPT_COLUMNS} WHERE excerpt.code = $1 AND excerpt.catalog_entry_id = $2"
            )))
            .bind(code.as_str())
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode_excerpt(&row, operation)).transpose()
        })
    }

    fn delete_excerpt<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "room_excerpt.delete";
            let deleted = sqlx::query(
                "DELETE FROM agent_room.room_excerpt WHERE code = $1 AND catalog_entry_id = $2",
            )
            .bind(code.as_str())
            .bind(catalog_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(deleted.rows_affected() > 0)
        })
    }

    fn prune_excerpts(
        &self,
        now: UtcMillis,
        default_retention_days: u16,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        Box::pin(async move {
            let operation = "room_excerpt.prune";
            let deleted = sqlx::query(
                r"DELETE FROM agent_room.room_excerpt AS excerpt
                   USING agent_room.room_catalog_entry AS catalog
                   WHERE catalog.id = excerpt.catalog_entry_id
                     AND (
                         excerpt.first_event_at < to_timestamp($1::double precision / 1000.0)
                             - make_interval(days => coalesce(catalog.retention_days, $2))
                         OR NOT EXISTS (
                             SELECT 1 FROM agent_room.room_live_view AS live
                              WHERE live.catalog_entry_id = excerpt.catalog_entry_id
                                AND live.enabled_at <= excerpt.first_event_at
                         )
                     )",
            )
            .bind(now.value())
            .bind(i32::from(default_retention_days))
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(deleted.rows_affected())
        })
    }
}

/// 锁着实况那一行再记：那一行得还在，还是 `enabled_at` 那一刻。
async fn insert_locked(
    transaction: &mut Transaction<'_, Postgres>,
    excerpt: &RoomExcerptRecord,
    enabled_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<InsertRoomExcerptOutcome> {
    let live: Option<i32> = sqlx::query_scalar(
        r"SELECT 1 FROM agent_room.room_live_view
           WHERE catalog_entry_id = $1
             AND floor(extract(epoch FROM enabled_at) * 1000)::bigint = $2
           FOR SHARE",
    )
    .bind(excerpt.catalog_id.as_uuid())
    .bind(enabled_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if live.is_none() {
        return Ok(InsertRoomExcerptOutcome::NotLive);
    }
    let event_ids: Vec<&str> = excerpt
        .event_ids
        .iter()
        .map(MatrixEventId::as_str)
        .collect();
    let inserted: Option<String> = sqlx::query_scalar(
        r"INSERT INTO agent_room.room_excerpt
              (code, catalog_entry_id, event_ids, first_event_at, shared_by, shared_at)
          VALUES ($1, $2, $3, to_timestamp($4::double precision / 1000.0), $5,
                  to_timestamp($6::double precision / 1000.0))
          ON CONFLICT (code) DO NOTHING
          RETURNING code",
    )
    .bind(excerpt.code.as_str())
    .bind(excerpt.catalog_id.as_uuid())
    .bind(&event_ids)
    .bind(excerpt.first_event_at.value())
    .bind(excerpt.shared_by.as_uuid())
    .bind(excerpt.shared_at.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(if inserted.is_some() {
        InsertRoomExcerptOutcome::Inserted
    } else {
        InsertRoomExcerptOutcome::CodeTaken
    })
}

/// 删掉一间房分享过的段。关实况的事务里，放在删实况那一行之后、单独一句（见本文件开头）。
pub(super) async fn delete_room_excerpts(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query("DELETE FROM agent_room.room_excerpt WHERE catalog_entry_id = $1")
        .bind(catalog_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_excerpt(row: &PgRow, operation: &'static str) -> RepositoryResult<RoomExcerptRecord> {
    let code: String = decode_column(row, "code", operation)?;
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let event_ids: Vec<String> = decode_column(row, "event_ids", operation)?;
    let shared_by: uuid::Uuid = decode_column(row, "shared_by", operation)?;
    Ok(RoomExcerptRecord {
        code: RoomExcerptCode::parse(&code).map_err(|_| corrupt_data(operation))?,
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        event_ids: event_ids
            .into_iter()
            .map(|event_id| MatrixEventId::new(event_id).map_err(|_| corrupt_data(operation)))
            .collect::<RepositoryResult<_>>()?,
        first_event_at: decode_time(row, "first_event_at_ms", operation)?,
        shared_by: PrincipalId::from_uuid(shared_by),
        shared_at: decode_time(row, "shared_at_ms", operation)?,
    })
}
