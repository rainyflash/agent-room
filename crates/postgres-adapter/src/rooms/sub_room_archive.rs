//! 子房间的归档和恢复（`specs/redesign/sub-rooms.md` 的“归档和恢复”，第 2e-2 步）。
//!
//! 和建子房间、改名一样先锁父房间在目录里的那一行：同一间房下面的归档、恢复、建、改名排着队来，
//! 恢复时重名和个数数得准；父房间这时候归档、转让也要等这一次存完，它跟着重算子房间时看得到这一次。
//! 房间的状态、父子关系上的记录、在等的申请、聊天服务器上要做的事、审计在同一个事务里。审计只记两个
//! 房间号和原因，名字是用户写的字，不进审计。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        PortFuture, SubRoomArchival, SubRoomArchiveOutcome, SubRoomArchiveStore, SubRoomRestoration,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoom,
    sub_room_archive::SubRoomArchiveReason,
    sub_rooms::MAXIMUM_OPEN_SUB_ROOMS,
    time::UtcMillis,
};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{PostgresRepositories, error::map_sqlx_error, transaction::finish};

use super::{
    join_requests::void_pending_of_room, private_rooms::write_private_room,
    sub_room_categories::lock_parent, sub_room_follow::queue_room_sync,
};

impl SubRoomArchiveStore for PostgresRepositories {
    fn archive_sub_room<'a>(
        &'a self,
        archival: &'a SubRoomArchival<'a>,
    ) -> PortFuture<'a, RepositoryResult<SubRoomArchiveOutcome>> {
        Box::pin(async move {
            let operation = "sub_room.archive";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = archive(&mut transaction, archival, operation).await;
            finish(transaction, result, operation).await
        })
    }

    fn restore_sub_room<'a>(
        &'a self,
        restoration: &'a SubRoomRestoration<'a>,
    ) -> PortFuture<'a, RepositoryResult<SubRoomArchiveOutcome>> {
        Box::pin(async move {
            let operation = "sub_room.restore";
            let mut transaction = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = restore(&mut transaction, restoration, operation).await;
            finish(transaction, result, operation).await
        })
    }
}

/// 父子关系那一行此刻的样子。
struct Link {
    archived: bool,
    reason: Option<String>,
    name_key: String,
}

async fn archive(
    transaction: &mut Transaction<'_, Postgres>,
    archival: &SubRoomArchival<'_>,
    operation: &'static str,
) -> RepositoryResult<SubRoomArchiveOutcome> {
    let sub_room = archival.room.catalog_id();
    if !lock_parent(transaction, archival.parent_catalog_id, operation).await? {
        return Ok(SubRoomArchiveOutcome::Unavailable);
    }
    let link = lock_link(transaction, sub_room, archival.parent_catalog_id, operation).await?;
    if link.is_none_or(|link| link.archived) {
        return Ok(SubRoomArchiveOutcome::Unavailable);
    }
    write_private_room(
        transaction,
        archival.room,
        archival.expected_version,
        archival.at,
        operation,
    )
    .await?;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = to_timestamp($2::double precision / 1000.0),
                 archived_by_principal_id = $3,
                 archived_reason = $4
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .bind(archival.at.value())
    .bind(archival.by.map(PrincipalId::as_uuid))
    .bind(archival.reason.as_str())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    // 归档了的子房间不收申请：在等的作废，要进的等它恢复以后再申请。
    void_pending_of_room(transaction, sub_room, archival.at, operation).await?;
    queue_room_sync(transaction, sub_room, archival.at, operation).await?;
    audit(
        transaction,
        &Audit {
            action: "sub_room.archived",
            actor: archival.by,
            room: archival.room,
            parent: archival.parent_catalog_id,
            reason: Some(archival.reason),
            at: archival.at,
        },
        operation,
    )
    .await?;
    Ok(SubRoomArchiveOutcome::Done)
}

async fn restore(
    transaction: &mut Transaction<'_, Postgres>,
    restoration: &SubRoomRestoration<'_>,
    operation: &'static str,
) -> RepositoryResult<SubRoomArchiveOutcome> {
    let sub_room = restoration.room.catalog_id();
    let parent = restoration.parent_catalog_id;
    if !lock_parent(transaction, parent, operation).await? {
        return Ok(SubRoomArchiveOutcome::Unavailable);
    }
    let Some(link) = lock_link(transaction, sub_room, parent, operation).await? else {
        return Ok(SubRoomArchiveOutcome::Unavailable);
    };
    // 跟着父房间归档的回不来：父房间这会儿在用，多半是以前的数据，照旧不动。
    let parent_reason = Some(SubRoomArchiveReason::Parent.as_str());
    if !link.archived || link.reason.as_deref() == parent_reason {
        return Ok(SubRoomArchiveOutcome::Unavailable);
    }
    let (open, same_name): (i64, bool) = sqlx::query_as(
        r"SELECT count(*)::bigint,
                 COALESCE(bool_or(name_key = $2), false)
            FROM agent_room.private_room_parent
           WHERE parent_catalog_entry_id = $1
             AND archived_at IS NULL
             AND catalog_entry_id <> $3",
    )
    .bind(parent.as_uuid())
    .bind(&link.name_key)
    .bind(sub_room.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if same_name {
        return Ok(SubRoomArchiveOutcome::NameTaken);
    }
    if open >= i64::from(MAXIMUM_OPEN_SUB_ROOMS) {
        return Ok(SubRoomArchiveOutcome::LimitReached);
    }
    write_private_room(
        transaction,
        restoration.room,
        restoration.expected_version,
        restoration.at,
        operation,
    )
    .await?;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = NULL,
                 archived_by_principal_id = NULL,
                 archived_reason = NULL
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    queue_room_sync(transaction, sub_room, restoration.at, operation).await?;
    audit(
        transaction,
        &Audit {
            action: "sub_room.restored",
            actor: Some(restoration.by),
            room: restoration.room,
            parent,
            reason: None,
            at: restoration.at,
        },
        operation,
    )
    .await?;
    Ok(SubRoomArchiveOutcome::Done)
}

/// 锁住父子关系那一行再读：父房间那一行已经锁着，改它的别的事务都排在后面。不挂在这间房下面为空。
async fn lock_link(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    parent: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<Option<Link>> {
    let row: Option<(bool, Option<String>, String)> = sqlx::query_as(
        r"SELECT archived_at IS NOT NULL, archived_reason, name_key
            FROM agent_room.private_room_parent
           WHERE catalog_entry_id = $1
             AND parent_catalog_entry_id = $2
             FOR NO KEY UPDATE",
    )
    .bind(sub_room.as_uuid())
    .bind(parent.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(row.map(|(archived, reason, name_key)| Link {
        archived,
        reason,
        name_key,
    }))
}

struct Audit<'a> {
    action: &'static str,
    /// 自动归档的没有。
    actor: Option<PrincipalId>,
    room: &'a PrivateRoom,
    parent: RoomCatalogId,
    reason: Option<SubRoomArchiveReason>,
    at: UtcMillis,
}

async fn audit(
    transaction: &mut Transaction<'_, Postgres>,
    audit: &Audit<'_>,
    operation: &'static str,
) -> RepositoryResult<()> {
    let id = Uuid::now_v7();
    let (actor_kind, actor_reference) = audit.actor.map_or_else(
        || ("service", "sub_room.auto_archive".to_owned()),
        |actor| ("principal", actor.to_string()),
    );
    let mut metadata = json!({ "parentCatalogId": audit.parent.to_string() });
    if let (Some(reason), Some(object)) = (audit.reason, metadata.as_object_mut()) {
        object.insert("reason".to_owned(), json!(reason.as_str()));
    }
    sqlx::query(
        r"INSERT INTO agent_room.audit_event (
               id, occurred_at, actor_kind, actor_reference, action,
               target_kind, target_reference, outcome, reason_code,
               correlation_id, metadata
           ) VALUES (
               $1, to_timestamp($2::double precision / 1000.0),
               $3, $4, $5, 'sub_room', $6, 'allowed', NULL, $1, $7
           )",
    )
    .bind(id)
    .bind(audit.at.value())
    .bind(actor_kind)
    .bind(actor_reference)
    .bind(audit.action)
    .bind(audit.room.catalog_id().to_string())
    .bind(metadata)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}
