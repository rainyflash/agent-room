//! 房间的常驻公告交给网络 Agent（`specs/public-rooms/design.md`“常驻公告”）：同步时把每个房间最新的
//! 一版记在 `network_agent_room` 上，再记交给它的是哪一版。进房间时交现在的，之后改了，交一批消息时
//! 交一次。和本机 Bridge 的消息库一样只往新的改：每次从头同步都会把同一版再给一遍，那不算改。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{MatrixRoomId, NetworkAgentInboxAppend, NetworkAgentRoomNotice},
};
use agent_room_domain::ids::NetworkAgentId;
use sqlx::{PgExecutor, Postgres, Transaction};

use crate::{agents::corrupt_data, error::map_sqlx_error};

/// 记下这一批同步到的公告，和收件箱同一个事务：位置对不上作废时一起作废。
pub(crate) async fn record_synced(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    operation: &'static str,
) -> RepositoryResult<()> {
    for notice in &append.room_notices {
        record(&mut **transaction, append.id, notice, false, operation).await?;
    }
    Ok(())
}

/// 进房间时交了这一版：记下它，也记成交过了。比记着的旧时什么都不改。
pub(crate) async fn give(
    executor: impl PgExecutor<'_>,
    id: NetworkAgentId,
    notice: &NetworkAgentRoomNotice,
) -> RepositoryResult<()> {
    record(executor, id, notice, true, "network_agent.give_room_notice").await
}

async fn record(
    executor: impl PgExecutor<'_>,
    id: NetworkAgentId,
    notice: &NetworkAgentRoomNotice,
    given: bool,
    operation: &'static str,
) -> RepositoryResult<()> {
    // 同步到的只认比记着的新的；进房间时读到的和记着的一样新也算交过（同步先到、还没交的那种）。
    sqlx::query(
        r"UPDATE agent_room.network_agent_room
             SET notice_text = $3,
                 notice_updated_by = $4,
                 notice_updated_at_ms = $5,
                 notice_given_at_ms = CASE WHEN $6 THEN $5 ELSE notice_given_at_ms END
           WHERE network_agent_id = $1 AND matrix_room_id = $2
             AND (notice_updated_at_ms IS NULL
                  OR notice_updated_at_ms < $5
                  OR ($6 AND notice_updated_at_ms = $5))",
    )
    .bind(id.as_uuid())
    .bind(notice.room_id.as_str())
    .bind(notice.text.as_deref())
    .bind(notice.updated_by.as_str())
    .bind(notice.updated_at_ms)
    .bind(given)
    .execute(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 拿改了的、记成交过了：从没交过的（撤掉的不算，没什么好说），或者交过以后又改了的（撤掉也算）。
/// 一条语句里挑出来再改掉，同一版只交一次。
pub(crate) async fn take(
    executor: impl PgExecutor<'_>,
    id: NetworkAgentId,
    room: Option<&MatrixRoomId>,
    limit: u16,
) -> RepositoryResult<Vec<NetworkAgentRoomNotice>> {
    let operation = "network_agent.take_room_notices";
    let rows = sqlx::query_as::<_, (String, Option<String>, String, i64)>(
        r"WITH changed AS (
              SELECT matrix_room_id
                FROM agent_room.network_agent_room
               WHERE network_agent_id = $1
                 AND ($2::text IS NULL OR matrix_room_id = $2)
                 AND notice_updated_at_ms IS NOT NULL
                 AND ((notice_given_at_ms IS NULL AND notice_text IS NOT NULL)
                      OR notice_given_at_ms < notice_updated_at_ms)
               ORDER BY joined_at, matrix_room_id
               LIMIT $3
               FOR UPDATE
          )
          UPDATE agent_room.network_agent_room room
             SET notice_given_at_ms = room.notice_updated_at_ms
            FROM changed
           WHERE room.network_agent_id = $1 AND room.matrix_room_id = changed.matrix_room_id
          RETURNING room.matrix_room_id, room.notice_text, room.notice_updated_by,
                    room.notice_updated_at_ms",
    )
    .bind(id.as_uuid())
    .bind(room.map(MatrixRoomId::as_str))
    .bind(i64::from(limit))
    .fetch_all(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let mut notices = rows
        .into_iter()
        .map(|(room_id, text, updated_by, updated_at_ms)| {
            Ok(NetworkAgentRoomNotice {
                room_id: MatrixRoomId::new(room_id).map_err(|_| corrupt_data(operation))?,
                text,
                updated_by,
                updated_at_ms,
            })
        })
        .collect::<RepositoryResult<Vec<_>>>()?;
    // RETURNING 不保证顺序：照房间排一下，每次一样。
    notices.sort_by(|left, right| left.room_id.as_str().cmp(right.room_id.as_str()));
    Ok(notices)
}
