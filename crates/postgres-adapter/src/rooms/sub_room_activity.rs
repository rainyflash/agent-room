//! 子房间最近有人说话的时间（`specs/redesign/sub-rooms.md` 第 2j-1 步）：和发现页一样每 5 分钟读一轮，
//! 写进父子关系那一行的 `last_message_at`。只往后挪；不算改子房间，不动房间的版本，管理员同时改设置
//! 不会撞上。
//!
//! 30 天没人说话自动归档（第 2j-2 步）每小时领一批：`FOR UPDATE SKIP LOCKED` 加上父子关系那一行的
//! 租期，开多个控制面时同一间不做两遍。归档本身走归档那一套（`sub_room_archive.rs`）。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{MatrixRoomId, PortFuture, QuietSubRoom, SubRoomActivityStore},
};
use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};
use uuid::Uuid;

use crate::{PostgresRepositories, error::map_sqlx_error};

use super::decode::corrupt_data;

impl SubRoomActivityStore for PostgresRepositories {
    fn sub_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>> {
        Box::pin(async move {
            let operation = "sub_room.activity.list";
            let rows: Vec<(Uuid, String)> = sqlx::query_as(
                r"SELECT link.catalog_entry_id, instance.matrix_room_id
                    FROM agent_room.private_room_parent AS link
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = link.catalog_entry_id
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = link.catalog_entry_id
                    JOIN agent_room.room_catalog_entry AS parent
                      ON parent.id = link.parent_catalog_entry_id
                   WHERE link.archived_at IS NULL
                     AND catalog.status = 'active'
                     AND instance.state = 'active'
                     AND parent.status = 'active'
                   ORDER BY link.catalog_entry_id",
            )
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.into_iter()
                .map(|(catalog_id, matrix_room)| {
                    let matrix_room =
                        MatrixRoomId::new(matrix_room).map_err(|_| corrupt_data(operation))?;
                    Ok((RoomCatalogId::from_uuid(catalog_id), matrix_room))
                })
                .collect()
        })
    }

    fn record_sub_room_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "sub_room.activity.record";
            sqlx::query(
                r"UPDATE agent_room.private_room_parent
                     SET last_message_at = greatest(
                             last_message_at,
                             to_timestamp($2::double precision / 1000.0)
                         )
                   WHERE catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .bind(at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn claim_quiet_sub_rooms(
        &self,
        quiet_before: UtcMillis,
        now: UtcMillis,
        lease_until: UtcMillis,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<QuietSubRoom>>> {
        Box::pin(async move {
            let operation = "sub_room.auto_archive.claim";
            // 没定过三项设置的照默认开着自动归档；只有 30 天这一种（表上的约束卡着）。greatest 不管空的：
            // 没说过话、没恢复过的按建的时间算。
            let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
                r"WITH due AS (
                      SELECT link.catalog_entry_id
                        FROM agent_room.private_room_parent AS link
                        JOIN agent_room.room_catalog_entry AS catalog
                          ON catalog.id = link.catalog_entry_id
                        JOIN agent_room.room_catalog_entry AS parent
                          ON parent.id = link.parent_catalog_entry_id
                        LEFT JOIN agent_room.private_room_sub_room_policy AS policy
                          ON policy.catalog_entry_id = link.parent_catalog_entry_id
                       WHERE link.archived_at IS NULL
                         AND catalog.status = 'active'
                         AND parent.status = 'active'
                         AND (policy.catalog_entry_id IS NULL
                              OR policy.auto_archive_days IS NOT NULL)
                         AND greatest(link.last_message_at, link.restored_at, link.created_at)
                             < to_timestamp($1::double precision / 1000.0)
                         AND (link.auto_archive_lease_until IS NULL
                              OR link.auto_archive_lease_until
                                 <= to_timestamp($2::double precision / 1000.0))
                       ORDER BY link.catalog_entry_id
                       LIMIT $4
                         FOR UPDATE OF link SKIP LOCKED
                  )
                  UPDATE agent_room.private_room_parent AS link
                     SET auto_archive_lease_until = to_timestamp($3::double precision / 1000.0)
                    FROM due
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = due.catalog_entry_id
                   WHERE link.catalog_entry_id = due.catalog_entry_id
                  RETURNING link.catalog_entry_id,
                            link.parent_catalog_entry_id,
                            instance.matrix_room_id",
            )
            .bind(quiet_before.value())
            .bind(now.value())
            .bind(lease_until.value())
            .bind(i64::from(limit))
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.into_iter()
                .map(|(catalog_id, parent_catalog_id, matrix_room)| {
                    Ok(QuietSubRoom {
                        catalog_id: RoomCatalogId::from_uuid(catalog_id),
                        parent_catalog_id: RoomCatalogId::from_uuid(parent_catalog_id),
                        matrix_room: MatrixRoomId::new(matrix_room)
                            .map_err(|_| corrupt_data(operation))?,
                    })
                })
                .collect()
        })
    }
}
