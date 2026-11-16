//! 房间的置顶交给网络 Agent（`specs/redesign/brief-and-pins.md`“Agent 读置顶”）：同步时把每个房间最新的
//! 一版记在 `network_agent_room` 上，再记交给它的是哪一版。规矩和公告一样（`network_agent_notices.rs`）：
//! 只往新的改，同一版只交一次；全取消了存成 NULL，从没交过的全取消了不说。那几句写了什么不存在这里，
//! 交的时候网关照它的消息记录。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        MatrixEventId, MatrixRoomId, NetworkAgentInboxAppend, NetworkAgentPin, NetworkAgentRoomPins,
    },
};
use agent_room_domain::ids::NetworkAgentId;
use serde_json::{Value, json};
use sqlx::{PgExecutor, Postgres, Transaction};

use crate::{agents::corrupt_data, error::map_sqlx_error};

/// 记下这一批同步到的置顶，和收件箱同一个事务：位置对不上作废时一起作废。
pub(crate) async fn record_synced(
    transaction: &mut Transaction<'_, Postgres>,
    append: &NetworkAgentInboxAppend,
    operation: &'static str,
) -> RepositoryResult<()> {
    for pins in &append.room_pins {
        record(&mut **transaction, append.id, pins, false, operation).await?;
    }
    Ok(())
}

/// 进房间时交了这一版：记下它，也记成交过了。比记着的旧时什么都不改。
pub(crate) async fn give(
    executor: impl PgExecutor<'_>,
    id: NetworkAgentId,
    pins: &NetworkAgentRoomPins,
) -> RepositoryResult<()> {
    record(executor, id, pins, true, "network_agent.give_room_pins").await
}

async fn record(
    executor: impl PgExecutor<'_>,
    id: NetworkAgentId,
    pins: &NetworkAgentRoomPins,
    given: bool,
    operation: &'static str,
) -> RepositoryResult<()> {
    let entries = (!pins.entries.is_empty()).then(|| encode(&pins.entries));
    sqlx::query(
        r"UPDATE agent_room.network_agent_room
             SET pins = $3,
                 pins_updated_by = $4,
                 pins_updated_at_ms = $5,
                 pins_given_at_ms = CASE WHEN $6 THEN $5 ELSE pins_given_at_ms END
           WHERE network_agent_id = $1 AND matrix_room_id = $2
             AND (pins_updated_at_ms IS NULL
                  OR pins_updated_at_ms < $5
                  OR ($6 AND pins_updated_at_ms = $5))",
    )
    .bind(id.as_uuid())
    .bind(pins.room_id.as_str())
    .bind(entries)
    .bind(pins.updated_by.as_str())
    .bind(pins.updated_at_ms)
    .bind(given)
    .execute(executor)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

/// 拿置顶改了的房间、记成交过了，一条语句里挑出来再改掉，同一版只交一次。
pub(crate) async fn take(
    pool: &sqlx::PgPool,
    id: NetworkAgentId,
    room: Option<&MatrixRoomId>,
    limit: u16,
) -> RepositoryResult<Vec<NetworkAgentRoomPins>> {
    let operation = "network_agent.take_room_pins";
    let rows = sqlx::query_as::<_, (String, Option<Value>, String, i64)>(
        r"WITH changed AS (
              SELECT matrix_room_id
                FROM agent_room.network_agent_room
               WHERE network_agent_id = $1
                 AND ($2::text IS NULL OR matrix_room_id = $2)
                 AND pins_updated_at_ms IS NOT NULL
                 AND ((pins_given_at_ms IS NULL AND pins IS NOT NULL)
                      OR pins_given_at_ms < pins_updated_at_ms)
               ORDER BY joined_at, matrix_room_id
               LIMIT $3
               FOR UPDATE
          )
          UPDATE agent_room.network_agent_room room
             SET pins_given_at_ms = room.pins_updated_at_ms
            FROM changed
           WHERE room.network_agent_id = $1 AND room.matrix_room_id = changed.matrix_room_id
          RETURNING room.matrix_room_id, room.pins, room.pins_updated_by,
                    room.pins_updated_at_ms",
    )
    .bind(id.as_uuid())
    .bind(room.map(MatrixRoomId::as_str))
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    let mut rooms = rows
        .into_iter()
        .map(|(room_id, entries, updated_by, updated_at_ms)| {
            Ok(NetworkAgentRoomPins {
                room_id: MatrixRoomId::new(room_id).map_err(|_| corrupt_data(operation))?,
                entries: decode(entries),
                updated_by,
                updated_at_ms,
            })
        })
        .collect::<RepositoryResult<Vec<_>>>()?;
    // RETURNING 不保证顺序：照房间排一下，每次一样。
    rooms.sort_by(|left, right| left.room_id.as_str().cmp(right.room_id.as_str()));
    Ok(rooms)
}

fn encode(entries: &[NetworkAgentPin]) -> Value {
    entries
        .iter()
        .map(|entry| {
            json!({
                "eventId": entry.event_id.as_str(),
                "pinnedBy": entry.pinned_by,
                "pinnedAtMs": entry.pinned_at_ms,
            })
        })
        .collect()
}

/// 读不懂的一条跳过。
fn decode(entries: Option<Value>) -> Vec<NetworkAgentPin> {
    let Some(Value::Array(entries)) = entries else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            Some(NetworkAgentPin {
                event_id: MatrixEventId::new(entry.get("eventId")?.as_str()?).ok()?,
                pinned_by: entry.get("pinnedBy")?.as_str()?.to_owned(),
                pinned_at_ms: entry.get("pinnedAtMs")?.as_i64()?,
            })
        })
        .collect()
}
