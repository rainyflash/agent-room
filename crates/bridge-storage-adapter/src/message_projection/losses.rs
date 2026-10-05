//! 时间线上补不回来的一段（`specs/agent-reading/design.md` 第 4 步的 `gaps`）。
//!
//! 同步时一个房间一次来得太多，Matrix 只给最近一段并标出 `limited`，Bridge 记下缺口再往回补。
//! 补到上限还没接上、或者根本没法往回补的，更早的那部分就丢了：记下它在哪两条之间，
//! 读收件箱读到后面那条时告诉 Agent。

use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::messages::{
    MessageBackfillBatch, MessageProjectionBatch, MessageProjectionMutation,
    MessageProjectionStoreFailure, MessageTimelineQueryFailure, TimelineLoss, TimelineLossReason,
};
use sqlx::{Row as _, Sqlite, Transaction};

use super::{SqliteMessageTimelineRepository, corrupt_query, map_query_sqlx_error, map_sqlx_error};

/// 记下这次同步里每个房间缺的那一段和它的两头。两头要在写入这次的消息以前看，之前最后一条才是对的。
/// 没有往回翻的令牌的补不回来，直接记成丢了的一段。
pub(super) async fn persist_gaps(
    transaction: &mut Transaction<'_, Sqlite>,
    batch: &MessageProjectionBatch,
) -> Result<(), MessageProjectionStoreFailure> {
    for gap in batch.gaps() {
        let after = latest_event(transaction, &gap.room_id).await?;
        let before = first_new_message(batch.mutations(), &gap.room_id);
        sqlx::query(
            "INSERT OR IGNORE INTO message_timeline_gap
             (sync_token, room_id, previous_batch, after_event_id, before_event_id)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(batch.next_batch().as_str())
        .bind(gap.room_id.as_str())
        .bind(
            gap.previous_batch
                .as_ref()
                .map_or("", |token| token.as_str()),
        )
        .bind(after.as_deref())
        .bind(before.map(MatrixEventId::as_str))
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        if gap.previous_batch.is_none()
            && let Some(before) = before
        {
            record_loss(transaction, &gap.room_id, after.as_deref(), before.as_str()).await?;
        }
    }
    Ok(())
}

/// 往回补没接上（翻到上限，或者房间不让读了）：更早的那部分补不回来，记成丢了的一段。
/// 它的后面一条是补回来的最早一条消息；一条也没补回来，就是当时同步来的第一条。
pub(super) async fn record_truncated_backfill(
    transaction: &mut Transaction<'_, Sqlite>,
    batch: &MessageBackfillBatch,
) -> Result<(), MessageProjectionStoreFailure> {
    if batch.complete() {
        return Ok(());
    }
    let gap = batch.gap();
    let recorded = sqlx::query(
        "SELECT after_event_id, before_event_id FROM message_timeline_gap
         WHERE sync_token = ? AND room_id = ? AND previous_batch = ?",
    )
    .bind(gap.sync_token.as_str())
    .bind(gap.room_id.as_str())
    .bind(gap.previous_batch.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    let (after, recorded_before) = recorded.map_or((None, None), |row| {
        (
            row.get::<Option<String>, _>("after_event_id"),
            row.get::<Option<String>, _>("before_event_id"),
        )
    });
    let before = first_new_message(batch.mutations(), &gap.room_id)
        .map(|event| event.as_str().to_owned())
        .or(recorded_before);
    // 升级前记下的缺口没有两头；一条也没补回来时就没处可挂，只能不报。
    if let Some(before) = before {
        record_loss(transaction, &gap.room_id, after.as_deref(), &before).await?;
    }
    Ok(())
}

async fn record_loss(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &MatrixRoomId,
    after: Option<&str>,
    before: &str,
) -> Result<(), MessageProjectionStoreFailure> {
    sqlx::query(
        "INSERT OR IGNORE INTO message_timeline_loss
         (room_id, before_event_id, after_event_id, reason)
         VALUES (?, ?, ?, ?)",
    )
    .bind(room_id.as_str())
    .bind(before)
    .bind(after)
    .bind(TimelineLossReason::TooMany.as_str())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    Ok(())
}

/// 这个房间里按时间最新的一条消息。按服务器收到的时间排：往回补的消息收得晚，时间却更早。
async fn latest_event(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &MatrixRoomId,
) -> Result<Option<String>, MessageProjectionStoreFailure> {
    sqlx::query_scalar::<_, String>(
        "SELECT base_event_id FROM message_current_projection
         WHERE room_id = ?
         ORDER BY origin_server_timestamp DESC, first_sequence DESC
         LIMIT 1",
    )
    .bind(room_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))
}

/// 这批里这个房间按时间最早的一条新消息。
fn first_new_message<'a>(
    mutations: &'a [MessageProjectionMutation],
    room_id: &MatrixRoomId,
) -> Option<&'a MatrixEventId> {
    mutations.iter().find_map(|mutation| match mutation {
        MessageProjectionMutation::Preview(preview) if &preview.room_id == room_id => {
            Some(&preview.event_id)
        }
        _ => None,
    })
}

impl SqliteMessageTimelineRepository {
    pub(super) async fn query_inbox_gaps(
        &self,
        room_id: &MatrixRoomId,
        event_ids: &[MatrixEventId],
    ) -> Result<Vec<TimelineLoss>, MessageTimelineQueryFailure> {
        if event_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = serde_json::to_string(
            &event_ids
                .iter()
                .map(MatrixEventId::as_str)
                .collect::<Vec<_>>(),
        )
        .map_err(|_| corrupt_query())?;
        let rows = sqlx::query(
            "SELECT before_event_id, after_event_id, reason FROM message_timeline_loss
             WHERE room_id = ? AND before_event_id IN (SELECT value FROM json_each(?))",
        )
        .bind(room_id.as_str())
        .bind(&wanted)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?;
        rows.iter()
            .map(|row| {
                Ok(TimelineLoss {
                    before_event_id: MatrixEventId::new(row.get::<String, _>("before_event_id"))
                        .map_err(|_| corrupt_query())?,
                    after_event_id: row
                        .get::<Option<String>, _>("after_event_id")
                        .map(MatrixEventId::new)
                        .transpose()
                        .map_err(|_| corrupt_query())?,
                    reason: TimelineLossReason::parse(&row.get::<String, _>("reason"))
                        .ok_or_else(corrupt_query)?,
                })
            })
            .collect()
    }
}
