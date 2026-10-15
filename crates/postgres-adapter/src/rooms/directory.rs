use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        PortFuture, PublicLobbyDirectoryEntry, PublicLobbyObservationRoom, PublicLobbyShard,
        PublicLobbyShardDirectory, RoomDirectory, RoomDirectoryQuery,
    },
};
use agent_room_domain::rooms::{MatrixRoomReference, RoomLanguage, RoomRegion};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId},
    rooms::RoomCatalog,
};

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

    fn find_public_observation_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>> {
        Box::pin(async move {
            let operation = "room_directory.find_public_observation_room";
            let row = sqlx::query(
                r"SELECT catalog.id AS catalog_id,
                         instance.id AS room_instance_id,
                         instance.matrix_room_id
                  FROM agent_room.room_catalog_entry AS catalog
                  JOIN agent_room.room_instance AS instance
                    ON instance.catalog_entry_id = catalog.id
                  WHERE catalog.id = $1
                    AND catalog.kind = 'public_lobby'
                    AND catalog.visibility = 'public'
                    AND catalog.status = 'active'
                    AND instance.state = 'active'
                  ORDER BY instance.allocated_slots DESC,
                           instance.created_at ASC,
                           instance.id ASC
                  LIMIT 1",
            )
            .bind(catalog_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            row.as_ref()
                .map(|row| {
                    let catalog_id: uuid::Uuid = decode_column(row, "catalog_id", operation)?;
                    let room_instance_id: uuid::Uuid =
                        decode_column(row, "room_instance_id", operation)?;
                    let matrix_room_id: String = decode_column(row, "matrix_room_id", operation)?;
                    Ok(PublicLobbyObservationRoom {
                        catalog_id: RoomCatalogId::from_uuid(catalog_id),
                        room_instance_id: RoomInstanceId::from_uuid(room_instance_id),
                        matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                            .map_err(|_| corrupt_data(operation))?,
                    })
                })
                .transpose()
        })
    }
}

impl PublicLobbyShardDirectory for PostgresRepositories {
    fn public_lobby_shards(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PublicLobbyShard>>> {
        Box::pin(async move {
            let operation = "room_directory.public_lobby_shards";
            let rows = sqlx::query_as::<_, (uuid::Uuid, String, i32, i64)>(
                r"SELECT instance.id, instance.matrix_room_id, instance.allocated_slots,
                         row_number() OVER (ORDER BY instance.created_at, instance.id)
                  FROM agent_room.room_catalog_entry AS catalog
                  JOIN agent_room.room_instance AS instance
                    ON instance.catalog_entry_id = catalog.id
                  WHERE catalog.id = $1
                    AND catalog.kind = 'public_lobby'
                    AND catalog.visibility = 'public'
                    AND catalog.status = 'active'
                    AND instance.state = 'active'
                  ORDER BY instance.created_at, instance.id",
            )
            .bind(catalog_id.as_uuid())
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.into_iter()
                .map(|(id, matrix_room_id, agents, number)| {
                    Ok(PublicLobbyShard {
                        number: u16::try_from(number).map_err(|_| corrupt_data(operation))?,
                        room_instance_id: RoomInstanceId::from_uuid(id),
                        matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                            .map_err(|_| corrupt_data(operation))?,
                        agents: u16::try_from(agents).map_err(|_| corrupt_data(operation))?,
                    })
                })
                .collect()
        })
    }
}
