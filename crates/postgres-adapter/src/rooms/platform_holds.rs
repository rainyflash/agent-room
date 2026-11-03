//! 平台把公开房间从目录里拿掉（`specs/public-rooms/design.md` 的 3b-4b）：一间房一行，放回去就删掉。
//! 房主的公开设置在 `private_room_listing` 里不动。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{PortFuture, PublicRoomPlatformHold, PublicRoomPlatformHoldStore},
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    error::{map_domain_error, map_sqlx_error},
};

impl PublicRoomPlatformHoldStore for PostgresRepositories {
    fn platform_hold(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicRoomPlatformHold>>> {
        Box::pin(async move {
            let operation = "public_room.platform_hold.find";
            let row: Option<(Uuid, String, i64)> = sqlx::query_as(
                r"SELECT held_by, reason, floor(extract(epoch FROM held_at) * 1000)::bigint
                    FROM agent_room.public_room_platform_hold
                   WHERE catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.map(|(held_by, reason, held_at)| {
                Ok(PublicRoomPlatformHold {
                    catalog_id,
                    held_by: PrincipalId::from_uuid(held_by),
                    reason,
                    held_at: UtcMillis::new(held_at)
                        .map_err(|error| map_domain_error(operation, &error))?,
                })
            })
            .transpose()
        })
    }

    fn place_platform_hold<'a>(
        &'a self,
        hold: &'a PublicRoomPlatformHold,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "public_room.platform_hold.place";
            // 已经拿掉的再拿一次：换成这一次的原因和人，时间只往后走。
            sqlx::query(
                r"INSERT INTO agent_room.public_room_platform_hold
                      (catalog_entry_id, held_by, reason, held_at)
                  VALUES ($1, $2, $3, to_timestamp($4::double precision / 1000.0))
                  ON CONFLICT (catalog_entry_id) DO UPDATE
                     SET held_by = excluded.held_by,
                         reason = excluded.reason,
                         held_at = greatest(
                             agent_room.public_room_platform_hold.held_at,
                             excluded.held_at
                         )",
            )
            .bind(hold.catalog_id.as_uuid())
            .bind(hold.held_by.as_uuid())
            .bind(&hold.reason)
            .bind(hold.held_at.value())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(())
        })
    }

    fn release_platform_hold(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "public_room.platform_hold.release";
            let deleted = sqlx::query(
                r"DELETE FROM agent_room.public_room_platform_hold
                   WHERE catalog_entry_id = $1",
            )
            .bind(catalog_id.as_uuid())
            .execute(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?
            .rows_affected();
            Ok(deleted > 0)
        })
    }
}
