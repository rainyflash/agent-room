//! 子房间跟着父房间变（`specs/redesign/sub-rooms.md` 的“对账”）。
//!
//! 父房间一存，同一个事务里就把受这次改动影响的子房间照父房间重算一遍：成员记录、房主、归没归档。
//! 这样库里任何时刻子房间都对得上父房间，按“这间房里他是不是成员”判断的地方不用各自再查父房间。
//! 重算的规则在领域层（`follow_parent_room`），这里只管读、写和排队。
//!
//! 聊天服务器那一半（踢人、改发言级别、邀请新房主、锁住归档的房间）事务里做不了：排进
//! `private_room_sub_room_sync`，事务外面照着做。
//!
//! 锁的先后：父房间的状态行，父房间在目录里的那一行，然后才是子房间的（状态行、目录、父子关系）。
//! 建子房间、给子房间改名先锁的也是父房间在目录里的那一行，所以它们和这里的互相排着队。

use std::collections::{BTreeMap, BTreeSet};

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{NewSubRoom, RoomRoleChange},
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus},
    room_role_changes::standing_changes,
    sub_rooms::follow_parent_room,
    time::UtcMillis,
    version::AggregateVersion,
};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
    transaction::finish,
};

use super::{
    decode::corrupt_data,
    private_rooms::{decode_member, write_private_room},
    role_changes::insert_role_changes,
};

/// 存父房间之前它的成员记录：主体、成员状态、权限的位。
pub(super) type ParentMembersBefore = BTreeMap<uuid::Uuid, (String, i16)>;

/// 读一间房时要不要锁它的状态行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateLock {
    /// 不锁：建子房间时父房间在目录里的那一行已经锁着，存父房间的进不来。
    None,
    /// 不让它这时候被存，正被别人锁着就当没读到：补对账时读父房间用，从不等锁。
    ShareIfFree,
    /// 接着要改它。
    Update,
}

/// 读出来的一间房，和它最近一次被写的时间。
struct LoadedRoom {
    room: PrivateRoom,
    /// 这间房的行里最晚的时间：跟着改的时候不能比它早（几张表都有“改的时间不早于建的时间”的约束，
    /// 存父房间的那一次用的钟可能比子房间上一次的慢）。
    touched_at: UtcMillis,
}

/// 这间房有子房间时，回它此刻（还没写这一次的成员）的成员记录；没有子房间回 `None`，后面什么都不用做。
///
/// 要在写完父房间在目录里的那一行以后调：那一行这时候锁着，正在这间房下面建的子房间要么已经提交、
/// 这里看得到，要么排在这次存完以后。
pub(super) async fn members_before_save(
    transaction: &mut Transaction<'_, Postgres>,
    parent: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<Option<ParentMembersBefore>> {
    let has_sub_rooms: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
              SELECT 1
                FROM agent_room.private_room_parent
               WHERE parent_catalog_entry_id = $1
          )",
    )
    .bind(parent.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if !has_sub_rooms {
        return Ok(None);
    }
    let rows: Vec<(uuid::Uuid, String, i16)> = sqlx::query_as(
        r"SELECT principal_id, membership_status, permission_bits
            FROM agent_room.private_room_membership
           WHERE catalog_entry_id = $1",
    )
    .bind(parent.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(Some(
        rows.into_iter()
            .map(|(principal, status, bits)| (principal, (status, bits)))
            .collect(),
    ))
}

/// 父房间刚存好（还在同一个事务里）：把受这次改动影响的子房间照它重算一遍。
///
/// 受影响的是：房主和父房间对不上的、父房间归档了自己还没归档的、里面有这次动到的人的。别的子房间
/// 这次不用动；原来就照到上一个版本的，直接记成照到了新版本。
pub(super) async fn follow_saved_parent(
    transaction: &mut Transaction<'_, Postgres>,
    parent: &PrivateRoom,
    previous_version: AggregateVersion,
    before: &ParentMembersBefore,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let moved: Vec<uuid::Uuid> = parent
        .members()
        .filter(|member| {
            before
                .get(&member.principal_id().as_uuid())
                .is_none_or(|(status, bits)| {
                    status != member.status().as_str()
                        || *bits != i16::from(member.permissions().bits())
                })
        })
        .map(|member| member.principal_id().as_uuid())
        .collect();
    let affected: Vec<uuid::Uuid> = sqlx::query_scalar(
        r"SELECT link.catalog_entry_id
            FROM agent_room.private_room_parent AS link
            JOIN agent_room.room_catalog_entry AS sub_room
              ON sub_room.id = link.catalog_entry_id
           WHERE link.parent_catalog_entry_id = $1
             AND (
                 sub_room.owner_principal_id IS DISTINCT FROM $2
                 OR ($3 AND sub_room.status = 'active')
                 OR EXISTS (
                     SELECT 1
                       FROM agent_room.private_room_membership AS member
                      WHERE member.catalog_entry_id = link.catalog_entry_id
                        AND member.principal_id = ANY($4)
                        AND member.membership_status IN ('invited', 'joined')
                 )
             )
           ORDER BY link.catalog_entry_id",
    )
    .bind(parent.catalog_id().as_uuid())
    .bind(parent.owner_principal_id().as_uuid())
    .bind(parent.status() == PrivateRoomLifecycleStatus::Archived)
    .bind(&moved)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    for sub_room in &affected {
        follow_one(
            transaction,
            parent,
            RoomCatalogId::from_uuid(*sub_room),
            changed_at,
            operation,
        )
        .await?;
    }
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET parent_version_applied = $2
           WHERE parent_catalog_entry_id = $1
             AND (parent_version_applied = $3 OR catalog_entry_id = ANY($4))",
    )
    .bind(parent.catalog_id().as_uuid())
    .bind(parent.version().value())
    .bind(previous_version.value())
    .bind(&affected)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let left: Vec<uuid::Uuid> = parent
        .members()
        .filter(|member| !member.has_joined() && moved.contains(&member.principal_id().as_uuid()))
        .map(|member| member.principal_id().as_uuid())
        .collect();
    void_requests_of_departed(transaction, parent, &left, changed_at, operation).await
}

/// 这次不再是父房间里已加入的人（退出、被移出、被封禁），他在各子房间在等的申请作废；父房间归档了的，
/// 子房间在等的申请都作废。申请的人不是子房间的成员，上面照着重算的子房间里没有他。
async fn void_requests_of_departed(
    transaction: &mut Transaction<'_, Postgres>,
    parent: &PrivateRoom,
    left: &[uuid::Uuid],
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let archived = parent.status() == PrivateRoomLifecycleStatus::Archived;
    if left.is_empty() && !archived {
        return Ok(());
    }
    sqlx::query(
        r"UPDATE agent_room.private_room_join_request AS request
             SET status = 'voided',
                 decided_at = greatest(
                     request.requested_at,
                     to_timestamp($4::double precision / 1000.0)
                 )
            FROM agent_room.private_room_parent AS link
           WHERE link.parent_catalog_entry_id = $1
             AND request.catalog_entry_id = link.catalog_entry_id
             AND request.status = 'pending'
             AND ($3 OR request.requester_principal_id = ANY($2))",
    )
    .bind(parent.catalog_id().as_uuid())
    .bind(left)
    .bind(archived)
    .bind(changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 子房间刚写进去（还在建它的那个事务里，父房间在目录里的那一行锁着）：它是照建之前读到的父房间
/// 算出来的，这期间父房间要是又存过，照现在的再算一遍。
pub(super) async fn follow_parent_at_creation(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: &NewSubRoom,
    operation: &'static str,
) -> RepositoryResult<()> {
    let parent = sub_room.parent_catalog_id;
    let version: Option<i64> = sqlx::query_scalar(
        r"SELECT version
            FROM agent_room.private_room_state
           WHERE catalog_entry_id = $1",
    )
    .bind(parent.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if version == Some(sub_room.parent_version.value()) {
        return Ok(());
    }
    let parent = load_room(transaction, parent, StateLock::None, operation)
        .await?
        .ok_or_else(|| corrupt_data(operation))?;
    let catalog_id = sub_room.snapshot.catalog().id();
    follow_one(
        transaction,
        &parent.room,
        catalog_id,
        sub_room.created_at,
        operation,
    )
    .await?;
    mark_followed(transaction, catalog_id, parent.room.version(), operation).await
}

/// 补对账：找出照到的版本比父房间旧的子房间（回滚到旧版那段时间里父房间改过的），一间一个事务地
/// 照父房间重算。回这一轮补了几间；父房间正被别人改着的这一轮不补、不算。一间出错不挡别的，最后把
/// 头一个错报出去。
pub(super) async fn follow_stale_sub_rooms(
    pool: &PgPool,
    limit: u16,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<u32> {
    let stale: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        r"SELECT link.catalog_entry_id, link.parent_catalog_entry_id
            FROM agent_room.private_room_parent AS link
            JOIN agent_room.private_room_state AS parent
              ON parent.catalog_entry_id = link.parent_catalog_entry_id
           WHERE link.parent_version_applied < parent.version
           ORDER BY link.parent_catalog_entry_id, link.catalog_entry_id
           LIMIT $1",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let mut followed = 0;
    let mut failure = None;
    for (sub_room, parent) in stale {
        let mut transaction = pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
        let result = follow_stale_one(
            &mut transaction,
            RoomCatalogId::from_uuid(sub_room),
            RoomCatalogId::from_uuid(parent),
            at,
            operation,
        )
        .await;
        match finish(transaction, result, operation).await {
            Ok(true) => followed += 1,
            Ok(false) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(followed), Err)
}

async fn follow_stale_one(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    parent: RoomCatalogId,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<bool> {
    // 和存父房间同一个先后：先它的状态行，再它在目录里的那一行。这两行锁着，存父房间的、在它下面
    // 建子房间和给子房间改名的都等这一间补完。哪一行正被别人锁着就不等、这一轮先不补：转让房主是
    // 先锁目录那一行再存的，等它会互相卡住。
    let Some(parent_room) =
        load_room(transaction, parent, StateLock::ShareIfFree, operation).await?
    else {
        return Ok(false);
    };
    let free = sqlx::query_scalar::<_, uuid::Uuid>(
        r"SELECT id
            FROM agent_room.room_catalog_entry
           WHERE id = $1
             FOR SHARE SKIP LOCKED",
    )
    .bind(parent.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if free.is_none() {
        return Ok(false);
    }
    let version = parent_room.room.version();
    let stale: Option<bool> = sqlx::query_scalar(
        r"SELECT parent_version_applied < $3
            FROM agent_room.private_room_parent
           WHERE catalog_entry_id = $1
             AND parent_catalog_entry_id = $2",
    )
    .bind(sub_room.as_uuid())
    .bind(parent.as_uuid())
    .bind(version.value())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if stale != Some(true) {
        // 等锁的时候别人已经补过了。
        return Ok(false);
    }
    follow_one(transaction, &parent_room.room, sub_room, at, operation).await?;
    mark_followed(transaction, sub_room, version, operation).await?;
    Ok(true)
}

/// 照 `parent` 把一间子房间重算一遍、存下，把聊天服务器上要跟着做的排上。没有要改的什么都不写。
async fn follow_one(
    transaction: &mut Transaction<'_, Postgres>,
    parent: &PrivateRoom,
    sub_room: RoomCatalogId,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let before = load_room(transaction, sub_room, StateLock::Update, operation)
        .await?
        .ok_or_else(|| corrupt_data(operation))?;
    let managers = sub_room_managers(transaction, sub_room, operation).await?;
    let Some(after) = follow_parent_room(parent, &before.room, &managers)
        .map_err(|error| map_domain_error(operation, &error))?
    else {
        return Ok(());
    };
    let at = changed_at.max(before.touched_at);
    write_private_room(transaction, &after, before.room.version(), at, operation).await?;
    // 这一次没有谁动手：是父房间变了带出来的。
    let changes: Vec<RoomRoleChange> = standing_changes(&before.room, &after)
        .into_iter()
        .map(|change| RoomRoleChange {
            actor: None,
            change,
        })
        .collect();
    insert_role_changes(transaction, sub_room, &changes, at, operation).await?;
    let moved: Vec<uuid::Uuid> = after
        .members()
        .filter(|member| before.room.member(member.principal_id()) != Some(*member))
        .map(|member| member.principal_id().as_uuid())
        .collect();
    queue_member_sync(transaction, sub_room, &moved, at, operation).await?;
    if before.room.status() == PrivateRoomLifecycleStatus::Active
        && after.status() == PrivateRoomLifecycleStatus::Archived
    {
        archive_with_parent(transaction, sub_room, at, operation).await?;
    }
    Ok(())
}

async fn load_room(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    lock: StateLock,
    operation: &'static str,
) -> RepositoryResult<Option<LoadedRoom>> {
    let locking = match lock {
        StateLock::None => "",
        StateLock::ShareIfFree => "FOR SHARE OF state SKIP LOCKED",
        StateLock::Update => "FOR UPDATE OF state",
    };
    let statement = format!(
        r"SELECT state.version,
                 state.created_by_agent_id,
                 catalog.owner_principal_id,
                 catalog.status,
                 floor(extract(epoch FROM greatest(
                     state.updated_at,
                     catalog.updated_at,
                     (SELECT max(member.created_at)
                        FROM agent_room.private_room_membership AS member
                       WHERE member.catalog_entry_id = state.catalog_entry_id)
                 )) * 1000)::bigint AS touched_at_ms
            FROM agent_room.private_room_state AS state
            JOIN agent_room.room_catalog_entry AS catalog
              ON catalog.id = state.catalog_entry_id
           WHERE state.catalog_entry_id = $1
             AND catalog.kind = 'private_room'
           {locking}"
    );
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(catalog_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?
    else {
        return Ok(None);
    };
    let version: i64 = decode_column(&row, "version", operation)?;
    let created_by_agent: Option<uuid::Uuid> =
        decode_column(&row, "created_by_agent_id", operation)?;
    let owner: Option<uuid::Uuid> = decode_column(&row, "owner_principal_id", operation)?;
    let status: String = decode_column(&row, "status", operation)?;
    let touched_at: i64 = decode_column(&row, "touched_at_ms", operation)?;
    // 冻结是公共大厅的状态，私人房间没有。
    let status = match status.as_str() {
        "active" => PrivateRoomLifecycleStatus::Active,
        "archived" => PrivateRoomLifecycleStatus::Archived,
        _ => return Err(corrupt_data(operation)),
    };
    let members = sqlx::query(
        r"SELECT membership.principal_id AS private_member_principal_id,
                 membership.membership_status AS private_member_status,
                 membership.permission_bits AS private_member_permission_bits
            FROM agent_room.private_room_membership AS membership
           WHERE membership.catalog_entry_id = $1
           ORDER BY membership.principal_id",
    )
    .bind(catalog_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?
    .iter()
    .map(|row| decode_member(row, operation))
    .collect::<RepositoryResult<Vec<_>>>()?;
    let room = PrivateRoom::restore(
        catalog_id,
        PrincipalId::from_uuid(owner.ok_or_else(|| corrupt_data(operation))?),
        status,
        members,
        AggregateVersion::new(version).map_err(|_| corrupt_data(operation))?,
        created_by_agent.map(AgentId::from_uuid),
    )
    .map_err(|_| corrupt_data(operation))?;
    Ok(Some(LoadedRoom {
        room,
        touched_at: UtcMillis::new(touched_at)
            .map_err(|error| map_domain_error(operation, &error))?,
    }))
}

async fn sub_room_managers(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<BTreeSet<PrincipalId>> {
    let managers: Vec<uuid::Uuid> = sqlx::query_scalar(
        r"SELECT principal_id
            FROM agent_room.private_room_sub_room_manager
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(managers.into_iter().map(PrincipalId::from_uuid).collect())
}

/// 这些人在这间子房间里的成员记录变了：排上，事务外面照现在的记录去改聊天服务器上的他们。已经排着的
/// 换成新的一版、从头再试。
async fn queue_member_sync(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    principals: &[uuid::Uuid],
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    if principals.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r"INSERT INTO agent_room.private_room_sub_room_sync AS queued (
               catalog_entry_id, principal_id, queued_at
           )
           SELECT $1::uuid, principal, to_timestamp($3::double precision / 1000.0)
             FROM unnest($2::uuid[]) AS principal
           ON CONFLICT (catalog_entry_id, principal_id) WHERE principal_id IS NOT NULL
           DO UPDATE
           SET generation = queued.generation + 1,
               queued_at = EXCLUDED.queued_at,
               attempts = 0,
               next_attempt_at = NULL,
               failure_code = NULL",
    )
    .bind(sub_room.as_uuid())
    .bind(principals)
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 子房间跟着父房间归档了：记下什么时候、为什么（没有“谁”），名字让出来；聊天服务器上锁住它的事
/// 排上。
async fn archive_with_parent(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = to_timestamp($2::double precision / 1000.0),
                 archived_reason = 'parent'
           WHERE catalog_entry_id = $1
             AND archived_at IS NULL",
    )
    .bind(sub_room.as_uuid())
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    queue_room_sync(transaction, sub_room, at, operation).await
}

/// 这间子房间自己的事（归档了要锁住，恢复了要解开）排上：事务外面照它那时候的状态去做。已经排着的
/// 换成新的一版、从头再试。
pub(super) async fn queue_room_sync(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.private_room_sub_room_sync AS queued (
               catalog_entry_id, principal_id, queued_at
           ) VALUES ($1, NULL, to_timestamp($2::double precision / 1000.0))
           ON CONFLICT (catalog_entry_id) WHERE principal_id IS NULL
           DO UPDATE
           SET generation = queued.generation + 1,
               queued_at = EXCLUDED.queued_at,
               attempts = 0,
               next_attempt_at = NULL,
               failure_code = NULL",
    )
    .bind(sub_room.as_uuid())
    .bind(at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn mark_followed(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    parent_version: AggregateVersion,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET parent_version_applied = $2
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .bind(parent_version.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}
