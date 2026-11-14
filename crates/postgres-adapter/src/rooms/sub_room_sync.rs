//! 子房间跟着父房间变了以后，聊天服务器上还没照着做的事（`private_room_sub_room_sync`）：领、做完、
//! 没做成往后排。往里排是改子房间的那个事务里的事，在 `sub_room_follow`。
//!
//! 一列管两件事：`next_attempt_at` 在做的时候是租约到期的时间，没做成以后是下次再试的时间，空着就是
//! 马上能领。重新排进来的那一版（`generation` 加一）把它清空，所以同一件事可能有两处同时在做：先领
//! 的那一处照的是旧样子，它在聊天服务器上的改动可能落在后领的那一处后面。所以做完、没做成的时候
//! 发现版本变了（或者这一条已经被别处做完删掉了），就再排一次，让它之后一定还有一遍照新样子做的。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{SubRoomSyncScope, SubRoomSyncTask},
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use sqlx::{PgPool, postgres::PgRow};

use crate::{agents::decode_column, error::map_sqlx_error};

use super::decode::corrupt_data;

pub(super) async fn claim(
    pool: &PgPool,
    scope: SubRoomSyncScope,
    limit: u16,
    now: UtcMillis,
    lease_until: UtcMillis,
) -> RepositoryResult<Vec<SubRoomSyncTask>> {
    const OPERATION: &str = "sub_room.sync.claim";
    let parent = match scope {
        SubRoomSyncScope::Everywhere => None,
        SubRoomSyncScope::Under(parent) => Some(parent.as_uuid()),
    };
    let rows = sqlx::query(
        r"WITH due AS (
              SELECT task.id
                FROM agent_room.private_room_sub_room_sync AS task
               WHERE (
                         task.next_attempt_at IS NULL
                         OR task.next_attempt_at <= to_timestamp($1::double precision / 1000.0)
                     )
                 AND (
                         $3::uuid IS NULL
                         OR task.catalog_entry_id IN (
                                SELECT link.catalog_entry_id
                                  FROM agent_room.private_room_parent AS link
                                 WHERE link.parent_catalog_entry_id = $3::uuid
                            )
                     )
               ORDER BY task.next_attempt_at NULLS FIRST, task.queued_at, task.id
               LIMIT $4
                 FOR UPDATE OF task SKIP LOCKED
          )
          UPDATE agent_room.private_room_sub_room_sync AS task
             SET attempts = task.attempts + 1,
                 next_attempt_at = to_timestamp($2::double precision / 1000.0)
            FROM due
           WHERE task.id = due.id
          RETURNING task.id, task.catalog_entry_id, task.principal_id, task.generation,
                    task.attempts",
    )
    .bind(now.value())
    .bind(lease_until.value())
    .bind(parent)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    let mut tasks = rows
        .iter()
        .map(|row| decode_task(row, OPERATION))
        .collect::<RepositoryResult<Vec<_>>>()?;
    // 返回的先后数据库不保证：照排进来的先后（编号）理一遍。
    tasks.sort_by_key(|task| task.id);
    Ok(tasks)
}

pub(super) async fn complete(pool: &PgPool, task: SubRoomSyncTask) -> RepositoryResult<()> {
    const OPERATION: &str = "sub_room.sync.complete";
    let done = sqlx::query(
        r"DELETE FROM agent_room.private_room_sub_room_sync
           WHERE id = $1 AND generation = $2",
    )
    .bind(task.id)
    .bind(task.generation)
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if done.rows_affected() == 0 {
        queue_again(pool, task, OPERATION).await?;
    }
    Ok(())
}

pub(super) async fn retry(
    pool: &PgPool,
    task: SubRoomSyncTask,
    retry_at: UtcMillis,
    failure_code: &'static str,
) -> RepositoryResult<()> {
    const OPERATION: &str = "sub_room.sync.retry";
    let postponed = sqlx::query(
        r"UPDATE agent_room.private_room_sub_room_sync
             SET next_attempt_at = to_timestamp($3::double precision / 1000.0),
                 failure_code = $4
           WHERE id = $1 AND generation = $2",
    )
    .bind(task.id)
    .bind(task.generation)
    .bind(retry_at.value())
    .bind(failure_code)
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if postponed.rows_affected() == 0 {
        queue_again(pool, task, OPERATION).await?;
    }
    Ok(())
}

/// 领到的那一版已经不在了：做的时候又排进来过。再排一次（马上能领），这一处照旧样子做的改动之后
/// 一定还有一遍照新样子做的。子房间连父子关系一起没了的不用排。
async fn queue_again(
    pool: &PgPool,
    task: SubRoomSyncTask,
    operation: &'static str,
) -> RepositoryResult<()> {
    // 一个人一条和一间房一条是两个唯一索引，撞上时各认各的。
    let statement = if task.principal.is_some() {
        r"INSERT INTO agent_room.private_room_sub_room_sync AS queued (
               catalog_entry_id, principal_id, queued_at
           )
           SELECT link.catalog_entry_id, $2::uuid, now()
             FROM agent_room.private_room_parent AS link
            WHERE link.catalog_entry_id = $1
           ON CONFLICT (catalog_entry_id, principal_id) WHERE principal_id IS NOT NULL
           DO UPDATE
           SET generation = queued.generation + 1,
               queued_at = EXCLUDED.queued_at,
               attempts = 0,
               next_attempt_at = NULL,
               failure_code = NULL"
    } else {
        r"INSERT INTO agent_room.private_room_sub_room_sync AS queued (
               catalog_entry_id, principal_id, queued_at
           )
           SELECT link.catalog_entry_id, $2::uuid, now()
             FROM agent_room.private_room_parent AS link
            WHERE link.catalog_entry_id = $1
           ON CONFLICT (catalog_entry_id) WHERE principal_id IS NULL
           DO UPDATE
           SET generation = queued.generation + 1,
               queued_at = EXCLUDED.queued_at,
               attempts = 0,
               next_attempt_at = NULL,
               failure_code = NULL"
    };
    sqlx::query(statement)
        .bind(task.catalog_id.as_uuid())
        .bind(task.principal.map(PrincipalId::as_uuid))
        .execute(pool)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_task(row: &PgRow, operation: &'static str) -> RepositoryResult<SubRoomSyncTask> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let principal: Option<uuid::Uuid> = decode_column(row, "principal_id", operation)?;
    let attempts: i32 = decode_column(row, "attempts", operation)?;
    Ok(SubRoomSyncTask {
        id: decode_column(row, "id", operation)?,
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        principal: principal.map(PrincipalId::from_uuid),
        generation: decode_column(row, "generation", operation)?,
        attempt: u32::try_from(attempts).map_err(|_| corrupt_data(operation))?,
    })
}
