use std::path::Path;

use agent_room_application::ports::{
    MatrixBackfillToken, MatrixEventId, MatrixRoomId, MatrixSyncToken, MatrixUserId, PortFuture,
};
use agent_room_bridge_core::agent_identity::BridgeAgentIdentity;
use agent_room_bridge_core::messages::{
    IsolatedSession, MessageBackfillBatch, MessageContentSourceQuery, MessagePreviewPage,
    MessagePreviewQuery, MessageProjectionBatch, MessageProjectionMutation,
    MessageProjectionStoreFailure, MessageProjectionStoreFailureKind, MessageRecoveryBatch,
    MessageRoomContext, MessageSyncIssue, MessageSyncIssueReason, MessageTimelineProjectionStore,
    MessageTimelineQueryFailure, MessageTimelineQueryFailureKind, MessageTimelineQueryRepository,
    OwnMembership, PendingTimelineGap, ProjectedActorInstanceVerification, ProjectedMessageActor,
    ProjectedMessagePreview, ReservedIsolatedEvent, RoomName, RoomStateChange,
    UndecryptableSession,
};
use agent_room_domain::{
    content::{ContentMediaType, Sha256Digest},
    ids::{AgentId, AgentInstanceId, ContentEncryptionContextId, ContentId, MessageId},
    messages::{
        CLIENT_CONTENT_KEY_BYTES, CLIENT_CONTENT_NONCE_BYTES, ClientContentEncryption,
        ClientContentEncryptionAlgorithm, MessageContentReference, MessageLanguage, MessagePreview,
        MessageProvenance, MessageRelation, MessageRiskFlag, MessageRiskFlags, MessageSensitivity,
        MessageSummary, MessageTitle,
    },
    time::UtcMillis,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::json;
use sqlx::{Row as _, Sqlite, SqlitePool, Transaction};
use uuid::{Uuid, Version};

use crate::{
    database::{SqliteBridgeStorageOpenFailure, begin_write, open_pool},
    error::{SqliteFailureKind, classify},
    message_projection_crypto::{
        MESSAGE_PROJECTION_WRAPPING_NONCE_BYTES, MessageProjectionKeyCipher,
        MessageProjectionStorageKey,
    },
};

#[derive(Clone)]
pub struct SqliteMessageTimelineRepository {
    pool: SqlitePool,
    key_cipher: MessageProjectionKeyCipher,
}

impl SqliteMessageTimelineRepository {
    /// 打开并迁移 Bridge 消息投影数据库。
    ///
    /// # Errors
    ///
    /// 目录不可创建、数据库不可连接或迁移失败时返回错误。
    pub async fn open(
        path: impl AsRef<Path>,
        storage_key: &MessageProjectionStorageKey,
    ) -> Result<Self, SqliteBridgeStorageOpenFailure> {
        open_pool(path.as_ref()).await.map(|pool| Self {
            pool,
            key_cipher: MessageProjectionKeyCipher::new(storage_key),
        })
    }

    async fn apply_batch(
        &self,
        batch: &MessageProjectionBatch,
    ) -> Result<(), MessageProjectionStoreFailure> {
        let mut transaction = begin_write(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        apply_timeline(
            &mut transaction,
            batch.next_batch(),
            batch.mutations(),
            batch.issues(),
            &self.key_cipher,
        )
        .await?;
        persist_gaps(&mut transaction, batch).await?;
        persist_cursor(&mut transaction, batch).await?;
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))
    }

    async fn apply_backfill_batch(
        &self,
        batch: &MessageBackfillBatch,
    ) -> Result<(), MessageProjectionStoreFailure> {
        let mut transaction = begin_write(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        // 补回的事件按时间先后追加在已收到的消息之后；已经记下的事件由事件 ID 去重。
        let gap = batch.gap();
        apply_timeline(
            &mut transaction,
            &gap.sync_token,
            batch.mutations(),
            batch.issues(),
            &self.key_cipher,
        )
        .await?;
        sqlx::query(
            "DELETE FROM message_timeline_gap
             WHERE sync_token = ? AND room_id = ? AND previous_batch = ?",
        )
        .bind(gap.sync_token.as_str())
        .bind(gap.room_id.as_str())
        .bind(gap.previous_batch.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))
    }

    async fn query_known_events(
        &self,
        room_id: &MatrixRoomId,
        event_ids: &[MatrixEventId],
    ) -> Result<Vec<MatrixEventId>, MessageProjectionStoreFailure> {
        if event_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = serde_json::to_string(
            &event_ids
                .iter()
                .map(MatrixEventId::as_str)
                .collect::<Vec<_>>(),
        )
        .map_err(|_| {
            MessageProjectionStoreFailure::new(MessageProjectionStoreFailureKind::Corrupt)
        })?;
        let known: Vec<String> = sqlx::query_scalar(
            "SELECT event_id FROM message_projection_event
             WHERE room_id = ? AND event_id IN (SELECT value FROM json_each(?))
             UNION
             SELECT event_id FROM message_sync_issue
             WHERE room_id = ? AND event_id IN (SELECT value FROM json_each(?))",
        )
        .bind(room_id.as_str())
        .bind(&wanted)
        .bind(room_id.as_str())
        .bind(&wanted)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        Ok(event_ids
            .iter()
            .filter(|id| known.iter().any(|known| known == id.as_str()))
            .cloned()
            .collect())
    }

    async fn query_pending_gaps(
        &self,
        limit: u16,
    ) -> Result<Vec<PendingTimelineGap>, MessageProjectionStoreFailure> {
        let rows = sqlx::query(
            "SELECT sync_token, room_id, previous_batch FROM message_timeline_gap
             WHERE previous_batch <> ''
             ORDER BY rowid ASC
             LIMIT ?",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        rows.iter()
            .map(|row| {
                let corrupt = || {
                    MessageProjectionStoreFailure::new(MessageProjectionStoreFailureKind::Corrupt)
                };
                Ok(PendingTimelineGap {
                    sync_token: MatrixSyncToken::new(row.get::<String, _>("sync_token"))
                        .map_err(|_| corrupt())?,
                    room_id: MatrixRoomId::new(row.get::<String, _>("room_id"))
                        .map_err(|_| corrupt())?,
                    previous_batch: MatrixBackfillToken::new(
                        row.get::<String, _>("previous_batch"),
                    )
                    .map_err(|_| corrupt())?,
                })
            })
            .collect()
    }

    async fn query_undecryptable_sessions(
        &self,
        limit: u16,
    ) -> Result<Vec<IsolatedSession>, MessageProjectionStoreFailure> {
        let rows = sqlx::query(
            "SELECT room_id, sender, sender_device, session_id,
                    MAX(reserved_sequence) AS latest
             FROM message_sync_issue
             WHERE reason = 'undecryptable' AND reserved_sequence IS NOT NULL
               AND sender IS NOT NULL AND session_id IS NOT NULL
             GROUP BY room_id, sender, sender_device, session_id
             ORDER BY latest DESC
             LIMIT ?",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        rows.iter()
            .map(|row| {
                Ok(IsolatedSession {
                    room_id: MatrixRoomId::new(row.get::<String, _>("room_id"))
                        .map_err(|_| corrupt_projection_failure())?,
                    session: UndecryptableSession {
                        sender: MatrixUserId::new(row.get::<String, _>("sender"))
                            .map_err(|_| corrupt_projection_failure())?,
                        sender_device: row.get("sender_device"),
                        session_id: row.get("session_id"),
                    },
                })
            })
            .collect()
    }

    async fn query_undecryptable_events(
        &self,
        room_id: &MatrixRoomId,
        session_ids: &[String],
        after: u64,
        limit: u16,
    ) -> Result<Vec<ReservedIsolatedEvent>, MessageProjectionStoreFailure> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted =
            serde_json::to_string(session_ids).map_err(|_| corrupt_projection_failure())?;
        let rows = sqlx::query(
            "SELECT event_id, reserved_sequence FROM message_sync_issue
             WHERE room_id = ? AND reason = 'undecryptable' AND reserved_sequence > ?
               AND session_id IN (SELECT value FROM json_each(?))
             ORDER BY reserved_sequence ASC
             LIMIT ?",
        )
        .bind(room_id.as_str())
        .bind(i64::try_from(after).map_err(|_| corrupt_projection_failure())?)
        .bind(&wanted)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
        rows.iter()
            .map(|row| {
                Ok(ReservedIsolatedEvent {
                    event_id: MatrixEventId::new(row.get::<String, _>("event_id"))
                        .map_err(|_| corrupt_projection_failure())?,
                    position: u64::try_from(row.get::<i64, _>("reserved_sequence"))
                        .map_err(|_| corrupt_projection_failure())?,
                })
            })
            .collect()
    }

    /// 重读出来的消息写在预留的序号上，再删掉隔离记录；别的原因不收的改记原因，不再占位置。
    async fn apply_recovery_batch(
        &self,
        batch: &MessageRecoveryBatch,
    ) -> Result<(), MessageProjectionStoreFailure> {
        let room_id = batch.room_id().as_str();
        let mut transaction = begin_write(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        for mutation in batch.recovered() {
            let event_id = mutation.event_id().as_str();
            if mutation.room_id() != batch.room_id() {
                return Err(corrupt_projection_failure());
            }
            let Some(sequence) = reserved_sequence(&mut transaction, room_id, event_id).await?
            else {
                continue;
            };
            let encoded = EncodedMutation::from_mutation(mutation, &self.key_cipher)?;
            write_mutation(&mut transaction, mutation, &encoded, sequence).await?;
            forget_undecryptable(&mut transaction, room_id, event_id).await?;
        }
        for issue in batch.reclassified() {
            let Some(event_id) = &issue.event_id else {
                continue;
            };
            sqlx::query(
                "UPDATE OR REPLACE message_sync_issue
                 SET reason = ?, reserved_sequence = NULL, sender = NULL, sender_device = NULL,
                     session_id = NULL
                 WHERE room_id = ? AND event_id = ? AND reason = 'undecryptable'",
            )
            .bind(issue.reason.as_str())
            .bind(room_id)
            .bind(event_id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        }
        for event_id in batch.dismissed() {
            forget_undecryptable(&mut transaction, room_id, event_id.as_str()).await?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))
    }
}

async fn reserved_sequence(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    event_id: &str,
) -> Result<Option<i64>, MessageProjectionStoreFailure> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MIN(reserved_sequence) FROM message_sync_issue
         WHERE room_id = ? AND event_id = ? AND reason = 'undecryptable'",
    )
    .bind(room_id)
    .bind(event_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))
}

async fn forget_undecryptable(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    event_id: &str,
) -> Result<(), MessageProjectionStoreFailure> {
    sqlx::query(
        "DELETE FROM message_sync_issue
         WHERE room_id = ? AND event_id = ? AND reason = 'undecryptable'",
    )
    .bind(room_id)
    .bind(event_id)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| map_sqlx_error(&error))
}

impl MessageTimelineProjectionStore for SqliteMessageTimelineRepository {
    fn apply<'a>(
        &'a self,
        batch: &'a MessageProjectionBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        Box::pin(async move { self.apply_batch(batch).await })
    }

    fn room_has_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<bool, MessageProjectionStoreFailure>> {
        Box::pin(async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM message_projection_event WHERE room_id = ?)",
            )
            .bind(room_id.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(&error))
        })
    }

    fn known_events<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_ids: &'a [MatrixEventId],
    ) -> PortFuture<'a, Result<Vec<MatrixEventId>, MessageProjectionStoreFailure>> {
        Box::pin(async move { self.query_known_events(room_id, event_ids).await })
    }

    fn pending_gaps(
        &self,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<PendingTimelineGap>, MessageProjectionStoreFailure>> {
        Box::pin(async move { self.query_pending_gaps(limit).await })
    }

    fn apply_backfill<'a>(
        &'a self,
        batch: &'a MessageBackfillBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        Box::pin(async move { self.apply_backfill_batch(batch).await })
    }

    fn undecryptable_sessions(
        &self,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<IsolatedSession>, MessageProjectionStoreFailure>> {
        Box::pin(async move { self.query_undecryptable_sessions(limit).await })
    }

    fn undecryptable_events<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        session_ids: &'a [String],
        after: u64,
        limit: u16,
    ) -> PortFuture<'a, Result<Vec<ReservedIsolatedEvent>, MessageProjectionStoreFailure>> {
        Box::pin(async move {
            self.query_undecryptable_events(room_id, session_ids, after, limit)
                .await
        })
    }

    fn apply_recovery<'a>(
        &'a self,
        batch: &'a MessageRecoveryBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        Box::pin(async move { self.apply_recovery_batch(batch).await })
    }

    fn sync_cursor(
        &self,
    ) -> PortFuture<'_, Result<Option<MatrixSyncToken>, MessageProjectionStoreFailure>> {
        Box::pin(async move {
            let stored: Option<String> =
                sqlx::query_scalar("SELECT next_batch FROM message_sync_state WHERE singleton = 1")
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|error| map_sqlx_error(&error))?;
            stored
                .map(|value| {
                    MatrixSyncToken::new(value).map_err(|_| {
                        MessageProjectionStoreFailure::new(
                            MessageProjectionStoreFailureKind::Corrupt,
                        )
                    })
                })
                .transpose()
        })
    }
}

impl MessageTimelineQueryRepository for SqliteMessageTimelineRepository {
    fn list_previews<'a>(
        &'a self,
        query: &'a MessagePreviewQuery,
    ) -> PortFuture<'a, Result<MessagePreviewPage, MessageTimelineQueryFailure>> {
        Box::pin(async move { self.query_previews(query).await })
    }

    fn find_content_source<'a>(
        &'a self,
        query: &'a MessageContentSourceQuery,
    ) -> PortFuture<'a, Result<Option<ProjectedMessagePreview>, MessageTimelineQueryFailure>> {
        Box::pin(async move { self.query_content_source(query).await })
    }

    fn find_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        message_ids: &'a [MessageId],
    ) -> PortFuture<'a, Result<Vec<ProjectedMessagePreview>, MessageTimelineQueryFailure>> {
        Box::pin(async move { self.query_messages(room_id, message_ids).await })
    }

    fn room_context<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<MessageRoomContext, MessageTimelineQueryFailure>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
                "SELECT name, joined_at_unix_ms FROM message_room_state WHERE room_id = ?",
            )
            .bind(room_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_query_sqlx_error(&error))?;
            Ok(
                row.map_or_else(MessageRoomContext::default, |(name, joined_at_ms)| {
                    MessageRoomContext { name, joined_at_ms }
                }),
            )
        })
    }
}

impl SqliteMessageTimelineRepository {
    /// 记下同步里看到的房间名和自己成员状态的变化：真正加入时记下时间，离开时清掉。
    ///
    /// # Errors
    ///
    /// 本地数据库写不进去时返回失败；调用方只记一条告警，不影响收消息。
    pub async fn record_room_state(
        &self,
        changes: &[RoomStateChange],
        now: UtcMillis,
    ) -> Result<(), MessageProjectionStoreFailure> {
        for change in changes {
            if let Some(name) = &change.name {
                let name = match name {
                    RoomName::Named(name) => Some(name.as_str()),
                    RoomName::Unnamed => None,
                };
                sqlx::query(
                    "INSERT INTO message_room_state (room_id, name, updated_at_unix_ms)
                     VALUES (?, ?, ?)
                     ON CONFLICT(room_id) DO UPDATE SET
                         name = excluded.name, updated_at_unix_ms = excluded.updated_at_unix_ms",
                )
                .bind(change.room_id.as_str())
                .bind(name)
                .bind(now.value())
                .execute(&self.pool)
                .await
                .map_err(|error| map_sqlx_error(&error))?;
            }
            if let Some(membership) = change.membership {
                let joined_at_ms = match membership {
                    OwnMembership::Joined { at_ms } => Some(at_ms),
                    OwnMembership::Left => None,
                };
                sqlx::query(
                    "INSERT INTO message_room_state (room_id, joined_at_unix_ms, updated_at_unix_ms)
                     VALUES (?, ?, ?)
                     ON CONFLICT(room_id) DO UPDATE SET
                         joined_at_unix_ms = excluded.joined_at_unix_ms,
                         updated_at_unix_ms = excluded.updated_at_unix_ms",
                )
                .bind(change.room_id.as_str())
                .bind(joined_at_ms)
                .bind(now.value())
                .execute(&self.pool)
                .await
                .map_err(|error| map_sqlx_error(&error))?;
            }
        }
        Ok(())
    }
}

impl SqliteMessageTimelineRepository {
    async fn query_previews(
        &self,
        query: &MessagePreviewQuery,
    ) -> Result<MessagePreviewPage, MessageTimelineQueryFailure> {
        let cursor_sequence = match query.after_event_id().or(query.before_event_id()) {
            Some(cursor) => Some(self.resolve_cursor(query.room_id(), cursor).await?),
            None => None,
        };
        let forward = query.oldest_first();
        let fetch_limit = i64::from(query.limit()) + 1;
        let rows = sqlx::query(
            "SELECT base_event_id, room_id, message_id, created_at_unix_ms,
                    origin_server_timestamp, actor_json, preview_json, content_json,
                    relation_target_message_id
             FROM message_current_projection
             WHERE room_id = ? AND visibility = 'active'
               AND (? IS NULL OR (? = 1 AND first_sequence > ?) OR (? = 0 AND first_sequence < ?))
             ORDER BY CASE WHEN ? = 1 THEN first_sequence END ASC,
                      CASE WHEN ? = 0 THEN first_sequence END DESC
             LIMIT ?",
        )
        .bind(query.room_id().as_str())
        .bind(cursor_sequence)
        .bind(forward)
        .bind(cursor_sequence)
        .bind(forward)
        .bind(cursor_sequence)
        .bind(forward)
        .bind(forward)
        .bind(fetch_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?;

        let has_more = rows.len() > usize::from(query.limit());
        let previews = rows
            .iter()
            .take(usize::from(query.limit()))
            .map(|row| decode_preview_row(row, &self.key_cipher))
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if has_more {
            previews.last().map(|preview| preview.event_id.clone())
        } else {
            None
        };
        Ok(MessagePreviewPage::new(previews, next_cursor))
    }

    async fn resolve_cursor(
        &self,
        room_id: &MatrixRoomId,
        cursor: &MatrixEventId,
    ) -> Result<i64, MessageTimelineQueryFailure> {
        sqlx::query_scalar::<_, i64>(
            "SELECT first_sequence
             FROM message_current_projection
             WHERE room_id = ? AND base_event_id = ?",
        )
        .bind(room_id.as_str())
        .bind(cursor.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?
        .ok_or_else(|| query_failure(MessageTimelineQueryFailureKind::CursorNotFound))
    }

    async fn query_messages(
        &self,
        room_id: &MatrixRoomId,
        message_ids: &[MessageId],
    ) -> Result<Vec<ProjectedMessagePreview>, MessageTimelineQueryFailure> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        // 一组 ID 编成一个 JSON 数组绑定，SQL 本身保持字面量。
        let wanted = serde_json::to_string(
            &message_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )
        .map_err(|_| corrupt_query())?;
        sqlx::query(
            "SELECT base_event_id, room_id, message_id, created_at_unix_ms,
                    origin_server_timestamp, actor_json, preview_json, content_json,
                    relation_target_message_id
             FROM message_current_projection
             WHERE room_id = ? AND visibility = 'active'
               AND message_id IN (SELECT value FROM json_each(?))",
        )
        .bind(room_id.as_str())
        .bind(wanted)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?
        .iter()
        .map(|row| decode_preview_row(row, &self.key_cipher))
        .collect()
    }

    async fn query_content_source(
        &self,
        query: &MessageContentSourceQuery,
    ) -> Result<Option<ProjectedMessagePreview>, MessageTimelineQueryFailure> {
        sqlx::query(
            "SELECT base_event_id, room_id, message_id, created_at_unix_ms,
                    origin_server_timestamp, actor_json, preview_json, content_json,
                    relation_target_message_id
             FROM message_current_projection
             WHERE room_id = ? AND content_id = ? AND visibility = 'active'",
        )
        .bind(query.room_id().as_str())
        .bind(query.content_id().to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?
        .as_ref()
        .map(|row| decode_preview_row(row, &self.key_cipher))
        .transpose()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredActor {
    agent_id: Option<String>,
    instance_id: Option<String>,
    principal_id: Option<String>,
    kind: Option<String>,
    display_name: String,
    matrix_user_id: String,
    avatar_url: Option<String>,
    provenance: String,
    instance_verification: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreview {
    conversation: Option<StoredConversation>,
    /// 收的时候已经按“是不是加密消息”判过，记下的就是算数的。
    #[serde(default)]
    mentions_everyone: bool,
    title: String,
    summary: String,
    content_type: String,
    language: Option<String>,
    sensitivity: String,
    risk_flags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredContent {
    content_id: String,
    digest_sha256: String,
    size_bytes: u64,
    encryption: Option<StoredClientContentEncryption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredClientContentEncryption {
    algorithm: String,
    context_id: String,
    wrapped_key_base64_url: String,
    wrapping_nonce_base64_url: String,
    nonce_base64_url: String,
    plaintext_size_bytes: u64,
}

fn decode_preview_row(
    row: &sqlx::sqlite::SqliteRow,
    key_cipher: &MessageProjectionKeyCipher,
) -> Result<ProjectedMessagePreview, MessageTimelineQueryFailure> {
    let actor = decode_actor(row.try_get("actor_json").map_err(|_| corrupt_query())?)?;
    let preview = decode_preview(row.try_get("preview_json").map_err(|_| corrupt_query())?)?;
    let room_id = MatrixRoomId::new(
        row.try_get::<String, _>("room_id")
            .map_err(|_| corrupt_query())?,
    )
    .map_err(|_| corrupt_query())?;
    let content = decode_content(
        row.try_get("content_json").map_err(|_| corrupt_query())?,
        &room_id,
        key_cipher,
    )?;
    let created_at = UtcMillis::new(
        row.try_get("created_at_unix_ms")
            .map_err(|_| corrupt_query())?,
    )
    .map_err(|_| corrupt_query())?;
    let origin_server_timestamp = row
        .try_get::<Option<i64>, _>("origin_server_timestamp")
        .map_err(|_| corrupt_query())?
        .map(u64::try_from)
        .transpose()
        .map_err(|_| corrupt_query())?;
    let relation = row
        .try_get::<Option<String>, _>("relation_target_message_id")
        .map_err(|_| corrupt_query())?
        .map(|value| {
            parse_v7(&value)
                .map(MessageId::from_uuid)
                .map(MessageRelation::ReplyTo)
        })
        .transpose()?;

    Ok(ProjectedMessagePreview {
        event_id: MatrixEventId::new(
            row.try_get::<String, _>("base_event_id")
                .map_err(|_| corrupt_query())?,
        )
        .map_err(|_| corrupt_query())?,
        transaction_id: None,
        room_id,
        message_id: MessageId::from_uuid(parse_v7(
            &row.try_get::<String, _>("message_id")
                .map_err(|_| corrupt_query())?,
        )?),
        created_at,
        origin_server_timestamp,
        actor,
        preview,
        content,
        relation,
    })
}

fn decode_actor(value: &str) -> Result<ProjectedMessageActor, MessageTimelineQueryFailure> {
    let stored = serde_json::from_str::<StoredActor>(value).map_err(|_| corrupt_query())?;
    if stored.kind.as_deref() == Some("human") {
        if stored.agent_id.is_some() || stored.instance_id.is_some() || stored.provenance != "human"
        {
            return Err(corrupt_query());
        }
        return Ok(ProjectedMessageActor::Human {
            principal_id: agent_room_domain::ids::PrincipalId::from_uuid(parse_v7(
                stored.principal_id.as_deref().ok_or_else(corrupt_query)?,
            )?),
            display_name: stored.display_name,
            matrix_user_id: MatrixUserId::new(stored.matrix_user_id)
                .map_err(|_| corrupt_query())?,
            avatar_url: stored.avatar_url,
        });
    }
    let mut identity = BridgeAgentIdentity::new(
        AgentId::from_uuid(parse_v7(
            stored.agent_id.as_deref().ok_or_else(corrupt_query)?,
        )?),
        stored.display_name,
        stored.matrix_user_id,
        AgentInstanceId::from_uuid(parse_v7(
            stored.instance_id.as_deref().ok_or_else(corrupt_query)?,
        )?),
    )
    .map_err(|_| corrupt_query())?;
    if let Some(avatar_url) = stored.avatar_url {
        identity = identity
            .with_avatar_url(avatar_url)
            .map_err(|_| corrupt_query())?;
    }
    let provenance =
        MessageProvenance::try_from(stored.provenance.as_str()).map_err(|_| corrupt_query())?;
    let verification = match stored.instance_verification.as_str() {
        "active" => ProjectedActorInstanceVerification::Active,
        "revoked_after_event" => ProjectedActorInstanceVerification::RevokedAfterEvent,
        _ => return Err(corrupt_query()),
    };
    Ok(ProjectedMessageActor::new(identity, provenance).with_instance_verification(verification))
}

fn decode_preview(value: &str) -> Result<MessagePreview, MessageTimelineQueryFailure> {
    let stored = serde_json::from_str::<StoredPreview>(value).map_err(|_| corrupt_query())?;
    let language = stored
        .language
        .map(MessageLanguage::new)
        .transpose()
        .map_err(|_| corrupt_query())?;
    let risk_flags = stored
        .risk_flags
        .into_iter()
        .map(MessageRiskFlag::new)
        .collect::<Result<Vec<_>, _>>()
        .and_then(MessageRiskFlags::new)
        .map_err(|_| corrupt_query())?;
    let mut result = MessagePreview::new(
        MessageTitle::new(stored.title).map_err(|_| corrupt_query())?,
        MessageSummary::new(stored.summary).map_err(|_| corrupt_query())?,
        ContentMediaType::new(stored.content_type).map_err(|_| corrupt_query())?,
        language,
        MessageSensitivity::try_from(stored.sensitivity.as_str()).map_err(|_| corrupt_query())?,
        risk_flags,
    );
    if let Some(chat) = stored.conversation {
        result = result.with_conversation(
            agent_room_domain::messages::ConversationMessage::new(chat.text, chat.mentions)
                .and_then(|value| value.with_attachment_name(chat.attachment_name))
                .map_err(|_| corrupt_query())?
                .with_mentions_everyone(stored.mentions_everyone),
        );
    }
    Ok(result)
}

fn decode_content(
    value: &str,
    room_id: &MatrixRoomId,
    key_cipher: &MessageProjectionKeyCipher,
) -> Result<MessageContentReference, MessageTimelineQueryFailure> {
    let stored = serde_json::from_str::<StoredContent>(value).map_err(|_| corrupt_query())?;
    let content_id = ContentId::from_uuid(parse_v7(&stored.content_id)?);
    let reference = MessageContentReference::new(
        content_id,
        Sha256Digest::from_bytes(decode_digest(&stored.digest_sha256)?),
        stored.size_bytes,
    )
    .map_err(|_| corrupt_query())?;
    let Some(stored_encryption) = stored.encryption else {
        return Ok(reference);
    };
    let algorithm =
        ClientContentEncryptionAlgorithm::try_from(stored_encryption.algorithm.as_str())
            .map_err(|_| corrupt_query())?;
    let encryption_context =
        ContentEncryptionContextId::from_uuid(parse_v7(&stored_encryption.context_id)?);
    let nonce =
        decode_base64_array::<CLIENT_CONTENT_NONCE_BYTES>(&stored_encryption.nonce_base64_url)?;
    let wrapping_nonce = decode_base64_array::<MESSAGE_PROJECTION_WRAPPING_NONCE_BYTES>(
        &stored_encryption.wrapping_nonce_base64_url,
    )?;
    let wrapped_key = URL_SAFE_NO_PAD
        .decode(&stored_encryption.wrapped_key_base64_url)
        .map_err(|_| corrupt_query())?;
    let placeholder = ClientContentEncryption::new(
        algorithm,
        encryption_context,
        [0_u8; CLIENT_CONTENT_KEY_BYTES],
        nonce,
        stored_encryption.plaintext_size_bytes,
    )
    .map_err(|_| corrupt_query())?;
    let key = key_cipher
        .unwrap(
            room_id,
            content_id,
            &placeholder,
            &wrapping_nonce,
            &wrapped_key,
        )
        .map_err(|()| corrupt_query())?;
    let encryption = ClientContentEncryption::new(
        algorithm,
        encryption_context,
        key,
        nonce,
        stored_encryption.plaintext_size_bytes,
    )
    .map_err(|_| corrupt_query())?;
    Ok(reference.with_client_encryption(encryption))
}

fn decode_base64_array<const LENGTH: usize>(
    value: &str,
) -> Result<[u8; LENGTH], MessageTimelineQueryFailure> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| corrupt_query())?
        .try_into()
        .map_err(|_| corrupt_query())
}

fn parse_v7(value: &str) -> Result<Uuid, MessageTimelineQueryFailure> {
    let id = Uuid::parse_str(value).map_err(|_| corrupt_query())?;
    if id.get_version() != Some(Version::SortRand) {
        return Err(corrupt_query());
    }
    Ok(id)
}

fn decode_digest(value: &str) -> Result<[u8; 32], MessageTimelineQueryFailure> {
    if value.len() != 64 {
        return Err(corrupt_query());
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = decode_hex_nibble(pair[0])?
            .checked_mul(16)
            .and_then(|high| high.checked_add(decode_hex_nibble(pair[1]).ok()?))
            .ok_or_else(corrupt_query)?;
    }
    Ok(digest)
}

fn decode_hex_nibble(value: u8) -> Result<u8, MessageTimelineQueryFailure> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(corrupt_query()),
    }
}

fn map_query_sqlx_error(error: &sqlx::Error) -> MessageTimelineQueryFailure {
    match classify(error) {
        SqliteFailureKind::Unavailable | SqliteFailureKind::Conflict => {
            query_failure(MessageTimelineQueryFailureKind::Unavailable)
        }
        SqliteFailureKind::Corrupt | SqliteFailureKind::NotFound => corrupt_query(),
    }
}

const fn query_failure(kind: MessageTimelineQueryFailureKind) -> MessageTimelineQueryFailure {
    MessageTimelineQueryFailure::new(kind)
}

const fn corrupt_query() -> MessageTimelineQueryFailure {
    query_failure(MessageTimelineQueryFailureKind::Corrupt)
}

/// 按观察顺序写一批事件：隔离记录插在它前面那些投影之后，解不开的事件就此占住位置。
async fn apply_timeline(
    transaction: &mut Transaction<'_, Sqlite>,
    sync_token: &MatrixSyncToken,
    mutations: &[MessageProjectionMutation],
    issues: &[MessageSyncIssue],
    key_cipher: &MessageProjectionKeyCipher,
) -> Result<(), MessageProjectionStoreFailure> {
    let mut issues = issues.iter().peekable();
    for (index, mutation) in mutations.iter().enumerate() {
        while let Some(issue) = issues.next_if(|issue| issue.mutations_before <= index) {
            persist_issue(transaction, sync_token, issue).await?;
        }
        apply_mutation(transaction, mutation, key_cipher).await?;
    }
    for issue in issues {
        persist_issue(transaction, sync_token, issue).await?;
    }
    Ok(())
}

async fn apply_mutation(
    transaction: &mut Transaction<'_, Sqlite>,
    mutation: &MessageProjectionMutation,
    key_cipher: &MessageProjectionKeyCipher,
) -> Result<(), MessageProjectionStoreFailure> {
    let encoded = EncodedMutation::from_mutation(mutation, key_cipher)?;
    let sequence = next_sequence(transaction, &encoded.room_id).await?;
    write_mutation(transaction, mutation, &encoded, sequence).await
}

async fn write_mutation(
    transaction: &mut Transaction<'_, Sqlite>,
    mutation: &MessageProjectionMutation,
    encoded: &EncodedMutation,
    sequence: i64,
) -> Result<(), MessageProjectionStoreFailure> {
    if insert_event(transaction, encoded, sequence).await? == 0 {
        return Ok(());
    }
    match mutation {
        MessageProjectionMutation::Preview(_) => {
            if insert_current(transaction, encoded, sequence).await? == 1 {
                apply_pending_revisions(transaction, &encoded.room_id, &encoded.message_id).await?;
            }
        }
        MessageProjectionMutation::Revision(_) => {
            apply_revision(transaction, encoded, sequence).await?;
        }
    }
    Ok(())
}

/// 下一个观察序号：已写入的事件和解不开的事件预留的序号都算在内。
async fn next_sequence(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> Result<i64, MessageProjectionStoreFailure> {
    sqlx::query_scalar::<_, i64>(
        "SELECT MAX(
            (SELECT COALESCE(MAX(sequence), 0) FROM message_projection_event WHERE room_id = ?),
            (SELECT COALESCE(MAX(reserved_sequence), 0) FROM message_sync_issue
             WHERE room_id = ? AND reserved_sequence IS NOT NULL)
         ) + 1",
    )
    .bind(room_id)
    .bind(room_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))
}

async fn insert_event(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &EncodedMutation,
    sequence: i64,
) -> Result<u64, MessageProjectionStoreFailure> {
    sqlx::query(
        "INSERT INTO message_projection_event (
            event_id, room_id, sequence, event_kind, message_id, revision_id,
            revision_kind, created_at_unix_ms, origin_server_timestamp,
            transaction_id, actor_subject_key, actor_json, preview_json,
            content_json, relation_target_message_id, content_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(&event.event_id)
    .bind(&event.room_id)
    .bind(sequence)
    .bind(event.event_kind)
    .bind(&event.message_id)
    .bind(event.revision_id.as_deref())
    .bind(event.revision_kind)
    .bind(event.created_at_unix_ms)
    .bind(event.origin_server_timestamp)
    .bind(event.transaction_id.as_deref())
    .bind(&event.actor_subject_key)
    .bind(&event.actor_json)
    .bind(event.preview_json.as_deref())
    .bind(event.content_json.as_deref())
    .bind(event.relation_target_message_id.as_deref())
    .bind(event.content_id.as_deref())
    .execute(&mut **transaction)
    .await
    .map(|result| result.rows_affected())
    .map_err(|error| map_sqlx_error(&error))
}

async fn insert_current(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &EncodedMutation,
    sequence: i64,
) -> Result<u64, MessageProjectionStoreFailure> {
    let preview_json = event
        .preview_json
        .as_deref()
        .ok_or_else(corrupt_projection_failure)?;
    let content_json = event
        .content_json
        .as_deref()
        .ok_or_else(corrupt_projection_failure)?;
    let content_id = event
        .content_id
        .as_deref()
        .ok_or_else(corrupt_projection_failure)?;
    sqlx::query(
        "INSERT INTO message_current_projection (
            message_id, room_id, base_event_id, first_sequence, last_sequence,
            created_at_unix_ms, origin_server_timestamp, actor_subject_key,
            actor_json, preview_json, content_json, relation_target_message_id,
            visibility, last_revision_event_id, content_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', NULL, ?)
         ON CONFLICT(message_id) DO NOTHING",
    )
    .bind(&event.message_id)
    .bind(&event.room_id)
    .bind(&event.event_id)
    .bind(sequence)
    .bind(sequence)
    .bind(event.created_at_unix_ms)
    .bind(event.origin_server_timestamp)
    .bind(&event.actor_subject_key)
    .bind(&event.actor_json)
    .bind(preview_json)
    .bind(content_json)
    .bind(event.relation_target_message_id.as_deref())
    .bind(content_id)
    .execute(&mut **transaction)
    .await
    .map(|result| result.rows_affected())
    .map_err(|error| map_sqlx_error(&error))
}

async fn apply_pending_revisions(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    message_id: &str,
) -> Result<(), MessageProjectionStoreFailure> {
    let rows = sqlx::query(
        "SELECT event_id, sequence, revision_kind, actor_subject_key,
                preview_json, content_json, content_id
         FROM message_projection_event
         WHERE room_id = ? AND message_id = ? AND event_kind = 'revision'
           AND revision_kind IN ('replace', 'redact')
         ORDER BY sequence ASC",
    )
    .bind(room_id)
    .bind(message_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    for row in rows {
        apply_revision_fields(
            transaction,
            RevisionFields {
                event_id: row
                    .try_get("event_id")
                    .map_err(|_| corrupt_projection_failure())?,
                room_id,
                message_id,
                sequence: row
                    .try_get("sequence")
                    .map_err(|_| corrupt_projection_failure())?,
                revision_kind: row
                    .try_get("revision_kind")
                    .map_err(|_| corrupt_projection_failure())?,
                actor_subject_key: row
                    .try_get("actor_subject_key")
                    .map_err(|_| corrupt_projection_failure())?,
                preview_json: row
                    .try_get("preview_json")
                    .map_err(|_| corrupt_projection_failure())?,
                content_json: row
                    .try_get("content_json")
                    .map_err(|_| corrupt_projection_failure())?,
                content_id: row
                    .try_get("content_id")
                    .map_err(|_| corrupt_projection_failure())?,
            },
        )
        .await?;
    }
    Ok(())
}

async fn apply_revision(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &EncodedMutation,
    sequence: i64,
) -> Result<(), MessageProjectionStoreFailure> {
    let revision_kind = event.revision_kind.ok_or_else(corrupt_projection_failure)?;
    apply_revision_fields(
        transaction,
        RevisionFields {
            event_id: event.event_id.clone(),
            room_id: &event.room_id,
            message_id: &event.message_id,
            sequence,
            revision_kind: revision_kind.to_owned(),
            actor_subject_key: event.actor_subject_key.clone(),
            preview_json: event.preview_json.clone(),
            content_json: event.content_json.clone(),
            content_id: event.content_id.clone(),
        },
    )
    .await
}

struct RevisionFields<'a> {
    event_id: String,
    room_id: &'a str,
    message_id: &'a str,
    sequence: i64,
    revision_kind: String,
    actor_subject_key: String,
    preview_json: Option<String>,
    content_json: Option<String>,
    content_id: Option<String>,
}

async fn apply_revision_fields(
    transaction: &mut Transaction<'_, Sqlite>,
    revision: RevisionFields<'_>,
) -> Result<(), MessageProjectionStoreFailure> {
    match revision.revision_kind.as_str() {
        "replace" => apply_replacement(transaction, &revision).await,
        "redact" => apply_redaction(transaction, &revision).await,
        "moderate" => Ok(()),
        _ => Err(corrupt_projection_failure()),
    }
}

async fn apply_replacement(
    transaction: &mut Transaction<'_, Sqlite>,
    revision: &RevisionFields<'_>,
) -> Result<(), MessageProjectionStoreFailure> {
    let preview_json = revision
        .preview_json
        .as_deref()
        .ok_or_else(corrupt_projection_failure)?;
    let content_json = revision
        .content_json
        .as_deref()
        .ok_or_else(corrupt_projection_failure)?;
    let content_id = revision
        .content_id
        .as_deref()
        .ok_or_else(corrupt_projection_failure)?;
    // 找回的旧修订写在它原来的位置；已经换上更新的修订时，不能再用旧的覆盖回去。
    sqlx::query(
        "UPDATE message_current_projection
         SET preview_json = ?, content_json = ?, content_id = ?, last_revision_event_id = ?,
             last_sequence = MAX(last_sequence, ?)
         WHERE room_id = ? AND message_id = ? AND actor_subject_key = ?
           AND visibility = 'active'
           AND NOT EXISTS (
               SELECT 1 FROM message_projection_event applied
               WHERE applied.event_id = message_current_projection.last_revision_event_id
                 AND applied.sequence > ?
           )",
    )
    .bind(preview_json)
    .bind(content_json)
    .bind(content_id)
    .bind(&revision.event_id)
    .bind(revision.sequence)
    .bind(revision.room_id)
    .bind(revision.message_id)
    .bind(&revision.actor_subject_key)
    .bind(revision.sequence)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| map_sqlx_error(&error))
}

async fn apply_redaction(
    transaction: &mut Transaction<'_, Sqlite>,
    revision: &RevisionFields<'_>,
) -> Result<(), MessageProjectionStoreFailure> {
    sqlx::query(
        "UPDATE message_current_projection
         SET content_json = NULL, content_id = NULL, visibility = 'redacted',
             last_revision_event_id = ?, last_sequence = MAX(last_sequence, ?)
         WHERE room_id = ? AND message_id = ? AND actor_subject_key = ?
           AND visibility = 'active'",
    )
    .bind(&revision.event_id)
    .bind(revision.sequence)
    .bind(revision.room_id)
    .bind(revision.message_id)
    .bind(&revision.actor_subject_key)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| map_sqlx_error(&error))
}

async fn persist_issue(
    transaction: &mut Transaction<'_, Sqlite>,
    sync_token: &MatrixSyncToken,
    issue: &MessageSyncIssue,
) -> Result<(), MessageProjectionStoreFailure> {
    let room_id = issue.room_id.as_str();
    let event_id = issue
        .event_id
        .as_ref()
        .map_or("", |event_id| event_id.as_str());
    let session = issue
        .session
        .as_ref()
        .filter(|_| issue.reason == MessageSyncIssueReason::Undecryptable && !event_id.is_empty());
    let reserved = match session {
        Some(_) => reserve_sequence(transaction, room_id, event_id).await?,
        None => None,
    };
    sqlx::query(
        "INSERT OR IGNORE INTO message_sync_issue
         (sync_token, room_id, event_id, reason, reserved_sequence, sender, sender_device,
          session_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(sync_token.as_str())
    .bind(room_id)
    .bind(event_id)
    .bind(issue.reason.as_str())
    .bind(reserved)
    .bind(session.map(|session| session.sender.as_str()))
    .bind(session.and_then(|session| session.sender_device.as_deref()))
    .bind(session.map(|session| session.session_id.as_str()))
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| map_sqlx_error(&error))
}

/// 给解不开的事件按观察顺序预留一个序号。重复同步到的（已经预留过）或已经写入的不再占。
async fn reserve_sequence(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    event_id: &str,
) -> Result<Option<i64>, MessageProjectionStoreFailure> {
    let known = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM message_projection_event WHERE event_id = ?)
             OR EXISTS(SELECT 1 FROM message_sync_issue
                       WHERE room_id = ? AND event_id = ? AND reserved_sequence IS NOT NULL)",
    )
    .bind(event_id)
    .bind(room_id)
    .bind(event_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(&error))?;
    if known {
        return Ok(None);
    }
    next_sequence(transaction, room_id).await.map(Some)
}

async fn persist_gaps(
    transaction: &mut Transaction<'_, Sqlite>,
    batch: &MessageProjectionBatch,
) -> Result<(), MessageProjectionStoreFailure> {
    for gap in batch.gaps() {
        sqlx::query(
            "INSERT OR IGNORE INTO message_timeline_gap
             (sync_token, room_id, previous_batch)
             VALUES (?, ?, ?)",
        )
        .bind(batch.next_batch().as_str())
        .bind(gap.room_id.as_str())
        .bind(
            gap.previous_batch
                .as_ref()
                .map_or("", |token| token.as_str()),
        )
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
    }
    Ok(())
}

async fn persist_cursor(
    transaction: &mut Transaction<'_, Sqlite>,
    batch: &MessageProjectionBatch,
) -> Result<(), MessageProjectionStoreFailure> {
    sqlx::query(
        "INSERT INTO message_sync_state (singleton, next_batch)
         VALUES (1, ?)
         ON CONFLICT(singleton) DO UPDATE SET next_batch = excluded.next_batch",
    )
    .bind(batch.next_batch().as_str())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| map_sqlx_error(&error))
}

struct EncodedMutation {
    event_id: String,
    room_id: String,
    event_kind: &'static str,
    message_id: String,
    revision_id: Option<String>,
    revision_kind: Option<&'static str>,
    created_at_unix_ms: i64,
    origin_server_timestamp: Option<i64>,
    transaction_id: Option<String>,
    actor_subject_key: String,
    actor_json: String,
    preview_json: Option<String>,
    content_json: Option<String>,
    content_id: Option<String>,
    relation_target_message_id: Option<String>,
}

impl EncodedMutation {
    fn from_mutation(
        mutation: &MessageProjectionMutation,
        key_cipher: &MessageProjectionKeyCipher,
    ) -> Result<Self, MessageProjectionStoreFailure> {
        match mutation {
            MessageProjectionMutation::Preview(preview) => Ok(Self {
                event_id: preview.event_id.as_str().to_owned(),
                room_id: preview.room_id.as_str().to_owned(),
                event_kind: "preview",
                message_id: preview.message_id.to_string(),
                revision_id: None,
                revision_kind: None,
                created_at_unix_ms: preview.created_at.value(),
                origin_server_timestamp: encode_server_timestamp(preview.origin_server_timestamp)?,
                transaction_id: preview
                    .transaction_id
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                actor_subject_key: preview.actor.subject_key(),
                actor_json: encode_actor(&preview.actor),
                preview_json: Some(encode_preview(&preview.preview)),
                content_json: Some(encode_content(
                    &preview.content,
                    &preview.room_id,
                    key_cipher,
                )?),
                content_id: Some(preview.content.content_id().to_string()),
                relation_target_message_id: preview.relation.map(relation_target),
            }),
            MessageProjectionMutation::Revision(revision) => Ok(Self {
                event_id: revision.event_id.as_str().to_owned(),
                room_id: revision.room_id.as_str().to_owned(),
                event_kind: "revision",
                message_id: revision.target_message_id.to_string(),
                revision_id: Some(revision.revision_id.to_string()),
                revision_kind: Some(revision.kind.as_str()),
                created_at_unix_ms: revision.created_at.value(),
                origin_server_timestamp: encode_server_timestamp(revision.origin_server_timestamp)?,
                transaction_id: revision
                    .transaction_id
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                actor_subject_key: revision.actor.subject_key(),
                actor_json: encode_actor(&revision.actor),
                preview_json: revision.preview.as_ref().map(encode_preview),
                content_json: revision
                    .content
                    .as_ref()
                    .map(|content| encode_content(content, &revision.room_id, key_cipher))
                    .transpose()?,
                content_id: revision
                    .content
                    .as_ref()
                    .map(|content| content.content_id().to_string()),
                relation_target_message_id: None,
            }),
        }
    }
}

fn encode_actor(actor: &ProjectedMessageActor) -> String {
    match actor {
        ProjectedMessageActor::Human { principal_id, display_name, matrix_user_id, avatar_url } => json!({
            "kind": "human", "principalId": principal_id.to_string(), "displayName": display_name,
            "matrixUserId": matrix_user_id.as_str(), "avatarUrl": avatar_url,
            "provenance": "human", "instanceVerification": "matrix_sender_matched",
        }),
        ProjectedMessageActor::Agent { identity, provenance, instance_verification } => json!({
            "kind": "agent", "agentId": identity.agent_id().to_string(),
            "instanceId": identity.agent_instance_id().to_string(), "displayName": identity.display_name(),
            "matrixUserId": identity.matrix_user_id().as_str(), "avatarUrl": identity.avatar_url(),
            "provenance": provenance.as_str(), "instanceVerification": instance_verification.as_str(),
        }),
    }.to_string()
}

fn encode_preview(preview: &MessagePreview) -> String {
    let mut encoded = json!({
        "conversation": preview.conversation().map(|chat| json!({"text": chat.text(), "mentions": chat.mentions(), "attachmentName": chat.attachment_name()})),
        "title": preview.title().as_str(),
        "summary": preview.summary().as_str(),
        "contentType": preview.content_type().as_str(),
        "language": preview.language().map(MessageLanguage::as_str),
        "sensitivity": preview.sensitivity().as_str(),
        "riskFlags": preview
            .risk_flags()
            .iter()
            .map(MessageRiskFlag::as_str)
            .collect::<Vec<_>>()
    });
    if preview
        .conversation()
        .is_some_and(agent_room_domain::messages::ConversationMessage::mentions_everyone)
    {
        encoded["mentionsEveryone"] = serde_json::Value::Bool(true);
    }
    encoded.to_string()
}

fn encode_content(
    content: &MessageContentReference,
    room_id: &MatrixRoomId,
    key_cipher: &MessageProjectionKeyCipher,
) -> Result<String, MessageProjectionStoreFailure> {
    let encryption = content
        .client_encryption()
        .map(|encryption| {
            let wrapped = key_cipher
                .wrap(room_id, content.content_id(), encryption)
                .map_err(|()| unavailable_projection_failure())?;
            Ok(json!({
                "algorithm": encryption.algorithm().as_str(),
                "contextId": encryption.context_id().to_string(),
                "wrappedKeyBase64Url": URL_SAFE_NO_PAD.encode(wrapped.ciphertext),
                "wrappingNonceBase64Url": URL_SAFE_NO_PAD.encode(wrapped.nonce),
                "nonceBase64Url": URL_SAFE_NO_PAD.encode(encryption.nonce()),
                "plaintextSizeBytes": encryption.plaintext_size_bytes()
            }))
        })
        .transpose()?;
    Ok(json!({
        "contentId": content.content_id().to_string(),
        "digestSha256": encode_hex(content.digest().as_bytes()),
        "sizeBytes": content.size_bytes(),
        "fetchMode": "on_demand",
        "encryption": encryption
    })
    .to_string())
}

fn relation_target(relation: MessageRelation) -> String {
    match relation {
        MessageRelation::ReplyTo(message_id) => message_id.to_string(),
    }
}

fn encode_server_timestamp(
    value: Option<u64>,
) -> Result<Option<i64>, MessageProjectionStoreFailure> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| corrupt_projection_failure())
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn map_sqlx_error(error: &sqlx::Error) -> MessageProjectionStoreFailure {
    let kind = match classify(error) {
        SqliteFailureKind::Conflict => MessageProjectionStoreFailureKind::Conflict,
        SqliteFailureKind::Corrupt | SqliteFailureKind::NotFound => {
            MessageProjectionStoreFailureKind::Corrupt
        }
        SqliteFailureKind::Unavailable => MessageProjectionStoreFailureKind::Unavailable,
    };
    MessageProjectionStoreFailure::new(kind)
}

const fn corrupt_projection_failure() -> MessageProjectionStoreFailure {
    MessageProjectionStoreFailure::new(MessageProjectionStoreFailureKind::Corrupt)
}

const fn unavailable_projection_failure() -> MessageProjectionStoreFailure {
    MessageProjectionStoreFailure::new(MessageProjectionStoreFailureKind::Unavailable)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredConversation {
    attachment_name: Option<String>,
    text: String,
    mentions: Vec<String>,
}
