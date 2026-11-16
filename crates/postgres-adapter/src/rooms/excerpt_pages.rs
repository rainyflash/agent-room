//! 打开分享的一段（`/r/<分享码>/s/<段码>`，3P-7a-2）：按分享码和段码一句 SQL 查齐。房间要和分享页实况
//! （`live_views.rs` 的 `LiveRoomLookup`）认的同几样，这一段还得是这次开实况以后的。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{ExcerptPageRecord, MatrixEventId, MatrixRoomId, PortFuture, RoomExcerptPageLookup},
};
use agent_room_domain::{
    ids::RoomCatalogId, room_excerpts::RoomExcerptCode, room_listing::RoomShareCode,
    time::UtcMillis,
};

use crate::{PostgresRepositories, error::map_sqlx_error};

use super::decode::corrupt_data;

type PageRow = (uuid::Uuid, String, String, i64, Vec<String>, i64);

impl RoomExcerptPageLookup for PostgresRepositories {
    fn excerpt_page<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<Option<ExcerptPageRecord>>> {
        Box::pin(async move {
            let operation = "room_excerpt.page";
            let row: Option<PageRow> = sqlx::query_as(
                r"SELECT catalog.id,
                         catalog.name,
                         instance.matrix_room_id,
                         floor(extract(epoch FROM live.enabled_at) * 1000)::bigint,
                         excerpt.event_ids,
                         floor(extract(epoch FROM excerpt.first_event_at) * 1000)::bigint
                    FROM agent_room.private_room_listing AS listing
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = listing.catalog_entry_id
                    JOIN agent_room.room_live_view AS live
                      ON live.catalog_entry_id = catalog.id
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = catalog.id
                    JOIN agent_room.room_excerpt AS excerpt
                      ON excerpt.catalog_entry_id = catalog.id
                   WHERE listing.share_code = $1
                     AND excerpt.code = $2
                     AND excerpt.first_event_at >= live.enabled_at
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
            .bind(code.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|row| decode(row, operation)).transpose()
        })
    }
}

fn decode(
    (catalog_id, room_name, matrix_room_id, enabled_at, event_ids, first_event_at): PageRow,
    operation: &'static str,
) -> RepositoryResult<ExcerptPageRecord> {
    Ok(ExcerptPageRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        room_name,
        matrix_room_id: MatrixRoomId::new(matrix_room_id).map_err(|_| corrupt_data(operation))?,
        enabled_at: UtcMillis::new(enabled_at).map_err(|_| corrupt_data(operation))?,
        event_ids: event_ids
            .into_iter()
            .map(|id| MatrixEventId::new(id).map_err(|_| corrupt_data(operation)))
            .collect::<RepositoryResult<_>>()?,
        first_event_at: UtcMillis::new(first_event_at).map_err(|_| corrupt_data(operation))?,
    })
}
