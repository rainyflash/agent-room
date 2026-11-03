//! 发现页的目录（`specs/public-rooms/design.md`）：公开着、还在用、平台没拿掉的私人房间。人数只数
//! 已加入的人；Agent 数和房间页一样，本机的有确认了的位置、网络的记着在这间房。

use std::collections::BTreeMap;

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        MatrixRoomId, MatrixUserId, PortFuture, PublicRoomActivityStore, PublicRoomDetailRecord,
        PublicRoomDirectoryStore, PublicRoomRecord, PublicRoomSearch, PublicRoomSort,
        PublicRoomViewerState,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
};

use super::decode::corrupt_data;

/// 详情里最多列几位管理员。
const MAX_LISTED_ADMINS: i64 = 10;

const ROOM_COLUMNS: &str = r"
    SELECT catalog.id,
           catalog.name,
           catalog.description,
           catalog.end_to_end_encrypted,
           floor(extract(epoch FROM catalog.created_at) * 1000)::bigint AS created_at_ms,
           listing.join_mode,
           listing.share_code,
           listing.first_listed_at,
           listing.last_message_at,
           floor(extract(epoch FROM listing.last_message_at) * 1000)::bigint AS last_message_at_ms,
           owner.id AS owner_principal_id,
           owner.display_name AS owner_display_name,
           (SELECT count(*)
              FROM agent_room.private_room_membership AS member
             WHERE member.catalog_entry_id = catalog.id
               AND member.membership_status = 'joined')::bigint AS member_count,
           (SELECT count(*) FROM (
                SELECT reservation.agent_id
                  FROM agent_room.room_capacity_reservation AS reservation
                 WHERE reservation.catalog_entry_id = catalog.id
                   AND reservation.state = 'committed'
                UNION
                SELECT network.agent_id
                  FROM agent_room.network_agent_room AS room
                  JOIN agent_room.network_agent AS network ON network.id = room.network_agent_id
                 WHERE room.catalog_entry_id = catalog.id
           ) AS present)::bigint AS agent_count
      FROM agent_room.private_room_listing AS listing
      JOIN agent_room.room_catalog_entry AS catalog ON catalog.id = listing.catalog_entry_id
      JOIN agent_room.principal AS owner ON owner.id = catalog.owner_principal_id
     WHERE listing.listed
       AND catalog.kind = 'private_room'
       AND catalog.status = 'active'
       AND NOT EXISTS (
           SELECT 1
             FROM agent_room.public_room_platform_hold AS hold
            WHERE hold.catalog_entry_id = catalog.id
       )";

/// 搜名字、简介和房主昵称 @ 前面那段；`$1` 是转义好的模糊匹配，空的就是不搜。
const SEARCH_FILTERS: &str = r"
       AND ($1::text IS NULL
            OR catalog.name ILIKE $1 ESCAPE '\'
            OR catalog.description ILIKE $1 ESCAPE '\'
            OR split_part(owner.display_name, '@', 1) ILIKE $1 ESCAPE '\')
       AND ($2::text IS NULL OR listing.join_mode = $2)
       AND ($3::bigint IS NULL
            OR listing.last_message_at >= to_timestamp($3::double precision / 1000.0))
       AND (NOT $4 OR NOT catalog.end_to_end_encrypted)";

/// 排序先按选的那一项，再按最近有人说话、最近公开，最后按房间 ID 定下来，翻页不乱。
const SEARCH_ORDER: &str = r"
     ORDER BY CASE WHEN $5 = 'members' THEN rooms.member_count END DESC NULLS LAST,
              CASE WHEN $5 = 'newest' THEN rooms.first_listed_at END DESC NULLS LAST,
              rooms.last_message_at DESC NULLS LAST,
              rooms.first_listed_at DESC,
              rooms.id
     LIMIT $6 OFFSET $7";

impl PublicRoomDirectoryStore for PostgresRepositories {
    fn search_public_rooms<'a>(
        &'a self,
        search: &'a PublicRoomSearch,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicRoomRecord>>> {
        Box::pin(async move {
            let operation = "public_room.search";
            let query =
                format!("SELECT * FROM ({ROOM_COLUMNS}{SEARCH_FILTERS}) AS rooms{SEARCH_ORDER}");
            let rows = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(search.text.as_deref().map(like_pattern))
                .bind(search.join_mode.map(RoomJoinMode::as_str))
                .bind(search.active_since.map(UtcMillis::value))
                .bind(search.unencrypted_only)
                .bind(sort_key(search.sort))
                .bind(i64::from(search.limit))
                .bind(i64::from(search.offset))
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            rows.iter().map(|row| decode_room(row, operation)).collect()
        })
    }

    fn public_room_by_share_code<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, RepositoryResult<Option<PublicRoomDetailRecord>>> {
        Box::pin(async move {
            let operation = "public_room.by_share_code";
            let query = format!("{ROOM_COLUMNS} AND listing.share_code = $1");
            let Some(row) = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(share_code.as_str())
                .fetch_optional(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?
            else {
                return Ok(None);
            };
            let room = decode_room(&row, operation)?;
            let owner: Uuid = decode_column(&row, "owner_principal_id", operation)?;
            // 有“管理”这一位、已加入的成员，房主不算；先进房间的在前。
            let admin_display_names: Vec<String> = sqlx::query_scalar(
                r"SELECT principal.display_name
                    FROM agent_room.private_room_membership AS member
                    JOIN agent_room.principal AS principal ON principal.id = member.principal_id
                   WHERE member.catalog_entry_id = $1
                     AND member.membership_status = 'joined'
                     AND member.permission_bits & 8 = 8
                     AND member.principal_id <> $2
                   ORDER BY member.created_at, member.principal_id
                   LIMIT $3",
            )
            .bind(room.catalog_id.as_uuid())
            .bind(owner)
            .bind(MAX_LISTED_ADMINS)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(Some(PublicRoomDetailRecord {
                room,
                admin_display_names,
            }))
        })
    }

    fn public_room_viewer_states<'a>(
        &'a self,
        viewer: PrincipalId,
        rooms: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<Vec<(RoomCatalogId, PublicRoomViewerState)>>> {
        Box::pin(async move {
            let operation = "public_room.viewer_states";
            let ids: Vec<Uuid> = rooms.iter().map(|room| room.as_uuid()).collect();
            let rows: Vec<(Uuid, String)> = sqlx::query_as(
                r"SELECT member.catalog_entry_id, member.membership_status
                    FROM agent_room.private_room_membership AS member
                   WHERE member.principal_id = $1
                     AND member.catalog_entry_id = ANY($2)
                     AND member.membership_status IN ('joined', 'invited', 'banned')
                  UNION ALL
                  SELECT request.catalog_entry_id, 'pending'
                    FROM agent_room.private_room_join_request AS request
                   WHERE request.requester_principal_id = $1
                     AND request.catalog_entry_id = ANY($2)
                     AND request.status = 'pending'",
            )
            .bind(viewer.as_uuid())
            .bind(&ids)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            // 在房间里（或被封禁）的处境盖过在等的申请：申请等着的时候被邀请进来了，就照邀请说。
            let mut states = BTreeMap::new();
            for (catalog_id, state) in rows {
                let state = match state.as_str() {
                    "joined" => PublicRoomViewerState::Joined,
                    "invited" => PublicRoomViewerState::Invited,
                    "banned" => PublicRoomViewerState::Banned,
                    "pending" => PublicRoomViewerState::Pending,
                    _ => return Err(corrupt_data(operation)),
                };
                let entry = states.entry(catalog_id).or_insert(state);
                if *entry == PublicRoomViewerState::Pending {
                    *entry = state;
                }
            }
            Ok(states
                .into_iter()
                .map(|(catalog_id, state)| (RoomCatalogId::from_uuid(catalog_id), state))
                .collect())
        })
    }

    fn public_room_presence_candidates(
        &self,
        catalog_id: RoomCatalogId,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<MatrixUserId>>> {
        Box::pin(async move {
            let operation = "public_room.presence_candidates";
            // 和人数、Agent 数数的是同一批；同一个 Agent 两边都记着时只算一次，按最近进来的那次排。
            let users: Vec<String> = sqlx::query_scalar(
                r"SELECT present.matrix_user_id
                    FROM (
                          SELECT principal.matrix_user_id, member.status_changed_at AS since
                            FROM agent_room.private_room_membership AS member
                            JOIN agent_room.principal AS principal
                              ON principal.id = member.principal_id
                           WHERE member.catalog_entry_id = $1
                             AND member.membership_status = 'joined'
                          UNION ALL
                          SELECT agent.matrix_user_id, reservation.finalized_at
                            FROM agent_room.room_capacity_reservation AS reservation
                            JOIN agent_room.agent AS agent ON agent.id = reservation.agent_id
                           WHERE reservation.catalog_entry_id = $1
                             AND reservation.state = 'committed'
                          UNION ALL
                          SELECT agent.matrix_user_id, room.joined_at
                            FROM agent_room.network_agent_room AS room
                            JOIN agent_room.network_agent AS network
                              ON network.id = room.network_agent_id
                            JOIN agent_room.agent AS agent ON agent.id = network.agent_id
                           WHERE room.catalog_entry_id = $1
                         ) AS present
                   GROUP BY present.matrix_user_id
                   ORDER BY max(present.since) DESC NULLS LAST, present.matrix_user_id
                   LIMIT $2",
            )
            .bind(catalog_id.as_uuid())
            .bind(i64::from(limit))
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            users
                .into_iter()
                .map(|user| MatrixUserId::new(user).map_err(|_| corrupt_data(operation)))
                .collect()
        })
    }
}

impl PublicRoomActivityStore for PostgresRepositories {
    fn listed_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>> {
        Box::pin(async move {
            let operation = "public_room.activity.list";
            let rows: Vec<(Uuid, String)> = sqlx::query_as(
                r"SELECT catalog.id, instance.matrix_room_id
                    FROM agent_room.private_room_listing AS listing
                    JOIN agent_room.room_catalog_entry AS catalog
                      ON catalog.id = listing.catalog_entry_id
                    JOIN agent_room.room_instance AS instance
                      ON instance.catalog_entry_id = catalog.id
                   WHERE listing.listed
                     AND catalog.kind = 'private_room'
                     AND catalog.status = 'active'
                     AND instance.state = 'active'
                     AND NOT EXISTS (
                         SELECT 1
                           FROM agent_room.public_room_platform_hold AS hold
                          WHERE hold.catalog_entry_id = catalog.id
                     )
                   ORDER BY catalog.id",
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

    fn record_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async move {
            let operation = "public_room.activity.record";
            // 不算改设置：不动版本和改的人，房主同时改设置不会撞上。
            sqlx::query(
                r"UPDATE agent_room.private_room_listing
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

/// 用户输入的字原样匹配：反斜杠、百分号和下划线先转义。
fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

const fn sort_key(sort: PublicRoomSort) -> &'static str {
    match sort {
        PublicRoomSort::Active => "active",
        PublicRoomSort::Members => "members",
        PublicRoomSort::Newest => "newest",
    }
}

fn decode_room(row: &PgRow, operation: &'static str) -> RepositoryResult<PublicRoomRecord> {
    let catalog_id: Uuid = decode_column(row, "id", operation)?;
    let join_mode: String = decode_column(row, "join_mode", operation)?;
    let share_code: String = decode_column(row, "share_code", operation)?;
    let created_at: i64 = decode_column(row, "created_at_ms", operation)?;
    let last_message_at: Option<i64> = decode_column(row, "last_message_at_ms", operation)?;
    let member_count: i64 = decode_column(row, "member_count", operation)?;
    let agent_count: i64 = decode_column(row, "agent_count", operation)?;
    let time = |value| UtcMillis::new(value).map_err(|error| map_domain_error(operation, &error));
    let count = |value| u32::try_from(value).map_err(|_| corrupt_data(operation));
    Ok(PublicRoomRecord {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        name: decode_column(row, "name", operation)?,
        description: decode_column(row, "description", operation)?,
        owner_display_name: decode_column(row, "owner_display_name", operation)?,
        member_count: count(member_count)?,
        agent_count: count(agent_count)?,
        join_mode: RoomJoinMode::try_from(join_mode.as_str())
            .map_err(|error| map_domain_error(operation, &error))?,
        end_to_end_encrypted: decode_column(row, "end_to_end_encrypted", operation)?,
        share_code: RoomShareCode::parse(&share_code)
            .map_err(|error| map_domain_error(operation, &error))?,
        created_at: time(created_at)?,
        last_message_at: last_message_at.map(time).transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::like_pattern;

    #[test]
    fn 搜的字原样匹配_通配符先转义() {
        assert_eq!(like_pattern("读书"), "%读书%");
        assert_eq!(like_pattern("50%_off\\"), "%50\\%\\_off\\\\%");
    }
}
