use agent_room_application::{
    persistence::RepositoryResult,
    ports::{PortFuture, PublicLobbyDirectoryEntry, RoomDirectory, RoomDirectoryQuery},
};
use agent_room_domain::rooms::{RoomLanguage, RoomRegion};
use agent_room_domain::{ids::RoomCatalogId, rooms::RoomCatalog};

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

use super::decode::{CATALOG_COLUMNS, corrupt_data, decode_catalog};

impl RoomDirectory for PostgresRepositories {
    fn list_public<'a>(
        &'a self,
        query: &'a RoomDirectoryQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
        Box::pin(async move {
            let operation = "room_directory.list_public";
            let statement = format!(
                r"SELECT {CATALOG_COLUMNS},
                          statistics.active_instance_count,
                          statistics.online_agent_count,
                          statistics.activity_score_millis
                   FROM agent_room.room_catalog_entry AS catalog
                   CROSS JOIN LATERAL (
                       SELECT count(instance.id)::bigint AS active_instance_count,
                              coalesce(sum(instance.member_count_projection), 0)::bigint
                                  AS online_agent_count,
                              coalesce(
                                  sum(floor(instance.activity_score * 1000)), 0
                              )::bigint AS activity_score_millis
                       FROM agent_room.room_instance AS instance
                       WHERE instance.catalog_entry_id = catalog.id
                         AND instance.state = 'active'
                   ) AS statistics
                   WHERE catalog.kind = 'public_lobby'
                     AND catalog.visibility = 'public'
                     AND catalog.status = 'active'
                     AND ($1::text IS NULL OR catalog.language = $1)
                     AND (
                         $2::text IS NULL
                         OR EXISTS (
                             SELECT 1
                             FROM agent_room.room_instance AS regional_instance
                             WHERE regional_instance.catalog_entry_id = catalog.id
                               AND regional_instance.state = 'active'
                               AND regional_instance.region_hint = $2
                         )
                     )
                   ORDER BY statistics.activity_score_millis DESC,
                            statistics.online_agent_count DESC,
                            catalog.slug ASC"
            );
            let language = query.language.as_ref().map(RoomLanguage::as_str);
            let region = query.region.as_ref().map(RoomRegion::as_str);
            // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
            let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
                .bind(language)
                .bind(region)
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;

            rows.iter()
                .map(|row| {
                    let active_instance_count: i64 =
                        decode_column(row, "active_instance_count", operation)?;
                    let online_agent_count: i64 =
                        decode_column(row, "online_agent_count", operation)?;
                    let activity_score_millis: i64 =
                        decode_column(row, "activity_score_millis", operation)?;
                    Ok(PublicLobbyDirectoryEntry {
                        catalog: decode_catalog(row, operation)?,
                        active_instance_count: u16::try_from(active_instance_count)
                            .map_err(|_| corrupt_data(operation))?,
                        online_agent_count: u32::try_from(online_agent_count)
                            .map_err(|_| corrupt_data(operation))?,
                        activity_score_millis: u64::try_from(activity_score_millis)
                            .map_err(|_| corrupt_data(operation))?,
                    })
                })
                .collect()
        })
    }

    fn find_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
        Box::pin(async move {
            let operation = "room_directory.find_catalog";
            let statement = format!(
                r"SELECT {CATALOG_COLUMNS}
                   FROM agent_room.room_catalog_entry AS catalog
                   WHERE catalog.id = $1"
            );
            // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
            let row = sqlx::query(sqlx::AssertSqlSafe(statement))
                .bind(catalog_id.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            row.as_ref()
                .map(|row| decode_catalog(row, operation))
                .transpose()
        })
    }
}
