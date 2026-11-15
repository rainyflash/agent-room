//! 父房间的禁言在子房间里一起算（`specs/redesign/sub-rooms.md` 的“禁言”）。这里是读的那一半：
//! 一个人在一间房里的禁言连父房间的一起读、他在哪些子房间里、他此刻是不是被禁言着。

use std::collections::HashMap;

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{MatrixRoomId, ModerationSubRoomStanding},
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    moderation::{ModerationAction, ModerationTarget, ModerationTargetKind},
    time::UtcMillis,
};
use sqlx::{PgExecutor, PgPool};

use crate::{agents::decode_column, error::map_sqlx_error};

use super::{ACTION_COLUMNS, corrupt_data, decode_action, principal_reference, same_target};

/// 这间房挂在哪间房下面；不是子房间时为 `None`。
pub(super) async fn parent_room<'e>(
    executor: impl PgExecutor<'e>,
    room_catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<Option<RoomCatalogId>> {
    let parent: Option<uuid::Uuid> = sqlx::query_scalar(
        r"SELECT parent_catalog_entry_id
            FROM agent_room.private_room_parent
           WHERE catalog_entry_id = $1",
    )
    .bind(room_catalog_id.as_uuid())
    .fetch_optional(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(parent.map(RoomCatalogId::from_uuid))
}

/// 这间房和它的父房间（有的话）。禁言按这一家子算。
async fn with_parent(
    pool: &PgPool,
    room_catalog_id: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<Vec<uuid::Uuid>> {
    let mut rooms = vec![room_catalog_id.as_uuid()];
    rooms.extend(
        parent_room(pool, room_catalog_id, operation)
            .await?
            .map(RoomCatalogId::as_uuid),
    );
    Ok(rooms)
}

/// 这些房间里已经落下和正在落的禁言，先做的在前。不分人：引用的写法不一定规范，不能拿文本比，读出来
/// 再按 UUID 挑；一间房的治理动作不多。
async fn mutes_in(
    pool: &PgPool,
    rooms: &[uuid::Uuid],
    operation: &'static str,
) -> RepositoryResult<Vec<ModerationAction>> {
    let statement = format!(
        "SELECT {ACTION_COLUMNS}
           FROM agent_room.moderation_action
           WHERE room_catalog_id = ANY($1)
             AND action_type = 'mute'
             AND status IN ('applied', 'pending')
           ORDER BY starts_at ASC, id ASC"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(rooms)
        .fetch_all(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| decode_action(row, operation))
        .collect()
}

/// 这个房间里这个人已经落下和正在落的禁言，先做的在前。这个房间是子房间时，他在父房间里的也在里面。
pub(super) async fn person_mutes(
    pool: &PgPool,
    room_catalog_id: RoomCatalogId,
    target: &ModerationTarget,
    operation: &'static str,
) -> RepositoryResult<Vec<ModerationAction>> {
    let rooms = with_parent(pool, room_catalog_id, operation).await?;
    Ok(mutes_in(pool, &rooms, operation)
        .await?
        .into_iter()
        .filter(|mute| same_target(target, mute.target().reference()))
        .collect())
}

/// 这个房间下面、没归档的子房间里，这个人此刻在里面（受邀或者已经加入）的，和他自己不在、他名下凭
/// 口令或敲门进来的 Agent 成员在里面的（2f-2b-2），先建的在前，各带着他在那一间里自己的禁言。对象
/// 不是人、这个房间下面没有子房间时是空的。
pub(super) async fn sub_room_standings(
    pool: &PgPool,
    room_catalog_id: RoomCatalogId,
    target: &ModerationTarget,
    operation: &'static str,
) -> RepositoryResult<Vec<ModerationSubRoomStanding>> {
    let Some(person) = principal_reference(target) else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query(
        r"SELECT link.catalog_entry_id, room.matrix_room_id
            FROM agent_room.private_room_parent AS link
            JOIN agent_room.room_instance AS room
              ON room.catalog_entry_id = link.catalog_entry_id
             AND room.state = 'active'
           WHERE link.parent_catalog_entry_id = $1
             AND link.archived_at IS NULL
             AND (
               EXISTS (
                 SELECT 1 FROM agent_room.private_room_membership AS membership
                  WHERE membership.catalog_entry_id = link.catalog_entry_id
                    AND membership.principal_id = $2
                    AND membership.membership_status IN ('invited', 'joined')
               )
               OR EXISTS (
                 SELECT 1 FROM agent_room.private_room_agent_member AS agent_member
                   JOIN agent_room.agent_ownership AS ownership
                     ON ownership.agent_id = agent_member.agent_id
                  WHERE agent_member.catalog_entry_id = link.catalog_entry_id
                    AND agent_member.membership_status = 'joined'
                    AND ownership.principal_id = $2
                    AND ownership.revoked_at IS NULL
               )
             )
           ORDER BY link.created_at ASC, link.catalog_entry_id ASC",
    )
    .bind(room_catalog_id.as_uuid())
    .bind(person)
    .fetch_all(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let sub_rooms = rows
        .iter()
        .map(|row| {
            let catalog: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
            let matrix_room: String = decode_column(row, "matrix_room_id", operation)?;
            Ok((catalog, matrix_room))
        })
        .collect::<RepositoryResult<Vec<_>>>()?;
    let catalogs: Vec<uuid::Uuid> = sub_rooms.iter().map(|(catalog, _)| *catalog).collect();
    let mut own: HashMap<uuid::Uuid, Vec<ModerationAction>> = HashMap::new();
    for mute in mutes_in(pool, &catalogs, operation).await? {
        if same_target(target, mute.target().reference()) {
            own.entry(mute.room_catalog_id().as_uuid())
                .or_default()
                .push(mute);
        }
    }
    sub_rooms
        .into_iter()
        .map(|(catalog, matrix_room)| {
            Ok(ModerationSubRoomStanding {
                matrix_room_id: MatrixRoomId::new(matrix_room)
                    .map_err(|_| corrupt_data(operation))?,
                mutes: own.remove(&catalog).unwrap_or_default(),
            })
        })
        .collect()
}

/// 这个人此刻在这个房间里是不是被禁言着：已经生效没到期的，正在落的也算；子房间里连父房间的一起算。
pub(super) async fn person_muted(
    pool: &PgPool,
    room_catalog_id: RoomCatalogId,
    principal_id: PrincipalId,
    now: UtcMillis,
) -> RepositoryResult<bool> {
    let operation = "moderation.person_muted";
    let rooms = with_parent(pool, room_catalog_id, operation).await?;
    Ok(mutes_in(pool, &rooms, operation).await?.iter().any(|mute| {
        mute.target().kind() == ModerationTargetKind::Principal
            && uuid::Uuid::parse_str(mute.target().reference())
                .is_ok_and(|person| person == principal_id.as_uuid())
            && mute.holds_at(now)
    }))
}
