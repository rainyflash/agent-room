//! 子房间里父房间须知的副本还没写的（`private_room_parent_notice_sync`，4N-3b）：排、领、写好、没写成往后
//! 排。新建子房间时在建房的那个事务里排一条，在 `sub_rooms.rs` 的 `insert_link`。
//!
//! 和 `sub_room_sync` 一样一列管两件事：`next_attempt_at` 在做的时候是租约到期的时间，没写成以后是下次
//! 再试的时间，空着就是马上能领。重新排进来的那一版（`generation` 加一）把它清空，所以同一间可能有两处
//! 同时在写：先领的那一处读到的是旧的那一版，它写的可能落在后领的那一处后面。所以写好、没写成的时候
//! 发现版本变了（或者这一条已经被别处做完删掉了），就再排一次，让它之后一定还有一遍照新的写的。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{ParentNoticeSyncStore, ParentNoticeSyncTask, PortFuture, SubRoomSyncScope},
};
use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgRow};

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

use super::decode::corrupt_data;

impl ParentNoticeSyncStore for PostgresRepositories {
    fn queue_parent_notice_sync(
        &self,
        parent: RoomCatalogId,
        queued_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        Box::pin(queue_under(self.pool(), parent, queued_at))
    }

    fn claim_parent_notice_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<ParentNoticeSyncTask>>> {
        Box::pin(claim(self.pool(), scope, limit, now, lease_until))
    }

    fn complete_parent_notice_sync(
        &self,
        task: ParentNoticeSyncTask,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(complete(self.pool(), task))
    }

    fn retry_parent_notice_sync(
        &self,
        task: ParentNoticeSyncTask,
        retry_at: UtcMillis,
        failure_code: &'static str,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(retry(self.pool(), task, retry_at, failure_code))
    }
}

/// 新建的子房间排一条：照父房间此刻的须知写上副本。和建子房间在同一个事务里。
pub(super) async fn queue_new_sub_room(
    transaction: &mut Transaction<'_, Postgres>,
    sub_room: RoomCatalogId,
    queued_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.private_room_parent_notice_sync (catalog_entry_id, queued_at)
          VALUES ($1, to_timestamp($2::double precision / 1000.0))",
    )
    .bind(sub_room.as_uuid())
    .bind(queued_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn queue_under(
    pool: &PgPool,
    parent: RoomCatalogId,
    queued_at: UtcMillis,
) -> RepositoryResult<u64> {
    const OPERATION: &str = "private_room.parent_notice_sync.queue";
    // 已经排着的重排：版本加一、马上能领、从头数，上一次没写成的原因清掉。
    let queued = sqlx::query(
        r"INSERT INTO agent_room.private_room_parent_notice_sync AS queued (
               catalog_entry_id, queued_at
           )
           SELECT link.catalog_entry_id, to_timestamp($2::double precision / 1000.0)
             FROM agent_room.private_room_parent AS link
            WHERE link.parent_catalog_entry_id = $1
           ON CONFLICT (catalog_entry_id) DO UPDATE
           SET generation = queued.generation + 1,
               queued_at = EXCLUDED.queued_at,
               attempts = 0,
               next_attempt_at = NULL,
               failure_code = NULL",
    )
    .bind(parent.as_uuid())
    .bind(queued_at.value())
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    Ok(queued.rows_affected())
}

async fn claim(
    pool: &PgPool,
    scope: SubRoomSyncScope,
    limit: u16,
    now: UtcMillis,
    lease_until: UtcMillis,
) -> RepositoryResult<Vec<ParentNoticeSyncTask>> {
    const OPERATION: &str = "private_room.parent_notice_sync.claim";
    let parent = match scope {
        SubRoomSyncScope::Everywhere => None,
        SubRoomSyncScope::Under(parent) => Some(parent.as_uuid()),
    };
    let rows = sqlx::query(
        r"WITH due AS (
              SELECT task.catalog_entry_id
                FROM agent_room.private_room_parent_notice_sync AS task
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
               ORDER BY task.next_attempt_at NULLS FIRST, task.queued_at, task.catalog_entry_id
               LIMIT $4
                 FOR UPDATE OF task SKIP LOCKED
          )
          UPDATE agent_room.private_room_parent_notice_sync AS task
             SET attempts = task.attempts + 1,
                 next_attempt_at = to_timestamp($2::double precision / 1000.0)
            FROM due
           WHERE task.catalog_entry_id = due.catalog_entry_id
          RETURNING task.catalog_entry_id, task.generation, task.attempts",
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
    // 返回的先后数据库不保证：按子房间的编号（UUIDv7，先建的在前）理一遍。
    tasks.sort_by_key(|task| task.catalog_id.as_uuid());
    Ok(tasks)
}

async fn complete(pool: &PgPool, task: ParentNoticeSyncTask) -> RepositoryResult<()> {
    const OPERATION: &str = "private_room.parent_notice_sync.complete";
    let done = sqlx::query(
        r"DELETE FROM agent_room.private_room_parent_notice_sync
           WHERE catalog_entry_id = $1 AND generation = $2",
    )
    .bind(task.catalog_id.as_uuid())
    .bind(task.generation)
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if done.rows_affected() == 0 {
        queue_again(pool, task.catalog_id, OPERATION).await?;
    }
    Ok(())
}

async fn retry(
    pool: &PgPool,
    task: ParentNoticeSyncTask,
    retry_at: UtcMillis,
    failure_code: &'static str,
) -> RepositoryResult<()> {
    const OPERATION: &str = "private_room.parent_notice_sync.retry";
    let postponed = sqlx::query(
        r"UPDATE agent_room.private_room_parent_notice_sync
             SET next_attempt_at = to_timestamp($3::double precision / 1000.0),
                 failure_code = $4
           WHERE catalog_entry_id = $1 AND generation = $2",
    )
    .bind(task.catalog_id.as_uuid())
    .bind(task.generation)
    .bind(retry_at.value())
    .bind(failure_code)
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(OPERATION, &error))?;
    if postponed.rows_affected() == 0 {
        queue_again(pool, task.catalog_id, OPERATION).await?;
    }
    Ok(())
}

/// 领到的那一版已经不在了：做的时候又排进来过。再排一次（马上能领），这一处照旧的那一版写的之后
/// 一定还有一遍照新的写的。子房间连父子关系一起没了的不用排。
async fn queue_again(
    pool: &PgPool,
    sub_room: RoomCatalogId,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.private_room_parent_notice_sync AS queued (
               catalog_entry_id, queued_at
           )
           SELECT link.catalog_entry_id, now()
             FROM agent_room.private_room_parent AS link
            WHERE link.catalog_entry_id = $1
           ON CONFLICT (catalog_entry_id) DO UPDATE
           SET generation = queued.generation + 1,
               queued_at = EXCLUDED.queued_at,
               attempts = 0,
               next_attempt_at = NULL,
               failure_code = NULL",
    )
    .bind(sub_room.as_uuid())
    .execute(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

fn decode_task(row: &PgRow, operation: &'static str) -> RepositoryResult<ParentNoticeSyncTask> {
    let catalog_id: uuid::Uuid = decode_column(row, "catalog_entry_id", operation)?;
    let attempts: i32 = decode_column(row, "attempts", operation)?;
    Ok(ParentNoticeSyncTask {
        catalog_id: RoomCatalogId::from_uuid(catalog_id),
        generation: decode_column(row, "generation", operation)?,
        attempt: u32::try_from(attempts).map_err(|_| corrupt_data(operation))?,
    })
}
