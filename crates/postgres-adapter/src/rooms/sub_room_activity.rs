//! 子房间最近有人说话的时间（`specs/redesign/sub-rooms.md` 第 2j-1 步）：和发现页一样每 5 分钟读一轮，
//! 写进父子关系那一行的 `last_message_at`。只往后挪；不算改子房间，不动房间的版本，管理员同时改设置
//! 不会撞上。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{MatrixRoomId, PortFuture, SubRoomActivityStore},
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
}
