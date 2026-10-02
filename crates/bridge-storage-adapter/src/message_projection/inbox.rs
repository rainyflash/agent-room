//! 收件箱的确认位置（`specs/agent-reading/design.md` 第 4 步）：每个房间记一条，只往前走。

use agent_room_application::ports::{MatrixEventId, MatrixRoomId, MatrixUserId};
use agent_room_bridge_core::messages::{
    InboxAcknowledgement, MessageLookupId, MessageTimelineQueryFailure,
    MessageTimelineQueryFailureKind,
};
use agent_room_domain::time::UtcMillis;

use super::{SqliteMessageTimelineRepository, corrupt_query, map_query_sqlx_error, query_failure};
use crate::database::begin_write;

impl SqliteMessageTimelineRepository {
    pub(super) async fn query_inbox_event(
        &self,
        id: &MessageLookupId,
    ) -> Result<Option<(MatrixRoomId, MatrixEventId)>, MessageTimelineQueryFailure> {
        let (event, message) = match id {
            MessageLookupId::Event(event) => (Some(event.as_str().to_owned()), None),
            MessageLookupId::Message(message) => (None, Some(message.to_string())),
        };
        // 撤回了的也认：收到以后才撤回的那条，照样能确认到它。
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT room_id, base_event_id
             FROM message_current_projection
             WHERE base_event_id = ? OR message_id = ?",
        )
        .bind(event)
        .bind(message)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?;
        row.map(|(room_id, event_id)| {
            Ok((
                MatrixRoomId::new(room_id).map_err(|_| corrupt_query())?,
                MatrixEventId::new(event_id).map_err(|_| corrupt_query())?,
            ))
        })
        .transpose()
    }

    pub(super) async fn query_inbox_position(
        &self,
        room_id: &MatrixRoomId,
    ) -> Result<Option<MatrixEventId>, MessageTimelineQueryFailure> {
        sqlx::query_scalar::<_, String>("SELECT event_id FROM message_inbox_ack WHERE room_id = ?")
            .bind(room_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_query_sqlx_error(&error))?
            .map(|event_id| MatrixEventId::new(event_id).map_err(|_| corrupt_query()))
            .transpose()
    }

    pub(super) async fn write_inbox_ack(
        &self,
        room_id: &MatrixRoomId,
        event_id: &MatrixEventId,
        viewer: &MatrixUserId,
        now: UtcMillis,
    ) -> Result<InboxAcknowledgement, MessageTimelineQueryFailure> {
        let mut transaction = begin_write(&self.pool)
            .await
            .map_err(|error| map_query_sqlx_error(&error))?;
        let sequence = sqlx::query_scalar::<_, i64>(
            "SELECT first_sequence
             FROM message_current_projection
             WHERE room_id = ? AND base_event_id = ?",
        )
        .bind(room_id.as_str())
        .bind(event_id.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?
        .ok_or_else(|| query_failure(MessageTimelineQueryFailureKind::CursorNotFound))?;
        let moved = sqlx::query(
            "INSERT INTO message_inbox_ack (room_id, event_id, sequence, updated_at_unix_ms)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(room_id) DO UPDATE SET
                 event_id = excluded.event_id,
                 sequence = excluded.sequence,
                 updated_at_unix_ms = excluded.updated_at_unix_ms
             WHERE excluded.sequence > message_inbox_ack.sequence",
        )
        .bind(room_id.as_str())
        .bind(event_id.as_str())
        .bind(sequence)
        .bind(now.value())
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?
        .rows_affected()
            > 0;
        // 还剩几条：位置之后别人发的、没撤回的。
        let pending = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)
             FROM message_current_projection
             WHERE room_id = ? AND visibility = 'active'
               AND first_sequence > (SELECT sequence FROM message_inbox_ack WHERE room_id = ?)
               AND json_extract(actor_json, '$.matrixUserId') IS NOT ?",
        )
        .bind(room_id.as_str())
        .bind(room_id.as_str())
        .bind(viewer.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?;
        transaction
            .commit()
            .await
            .map_err(|error| map_query_sqlx_error(&error))?;
        Ok(InboxAcknowledgement {
            acknowledged: moved,
            pending: u64::try_from(pending).map_err(|_| corrupt_query())?,
        })
    }
}
