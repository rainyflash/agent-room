use std::path::Path;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::messages::{
    MessageProjectionBatch, MessageProjectionMutation, MessageProjectionStoreFailure,
    MessageProjectionStoreFailureKind, MessageTimelineProjectionStore, ProjectedMessageActor,
};
use agent_room_domain::messages::{MessageContentReference, MessagePreview, MessageRelation};
use serde_json::json;
use sqlx::{Row as _, Sqlite, SqlitePool, Transaction};

use crate::{
    database::{SqliteBridgeStorageOpenFailure, open_pool},
    error::{SqliteFailureKind, classify},
};

#[derive(Clone)]
pub struct SqliteMessageTimelineRepository {
    pool: SqlitePool,
}

impl SqliteMessageTimelineRepository {
    /// 打开并迁移 Bridge 消息投影数据库。
    ///
    /// # Errors
    ///
    /// 目录不可创建、数据库不可连接或迁移失败时返回错误。
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SqliteBridgeStorageOpenFailure> {
        open_pool(path.as_ref()).await.map(|pool| Self { pool })
    }

    async fn apply_batch(
        &self,
        batch: &MessageProjectionBatch,
    ) -> Result<(), MessageProjectionStoreFailure> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(&error))?;
        for mutation in batch.mutations() {
            apply_mutation(&mut transaction, mutation).await?;
        }
        persist_issues(&mut transaction, batch).await?;
        persist_gaps(&mut transaction, batch).await?;
        persist_cursor(&mut transaction, batch).await?;
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(&error))
    }
}

impl MessageTimelineProjectionStore for SqliteMessageTimelineRepository {
    fn apply<'a>(
        &'a self,
        batch: &'a MessageProjectionBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        Box::pin(async move { self.apply_batch(batch).await })
    }
}

async fn apply_mutation(
    transaction: &mut Transaction<'_, Sqlite>,
    mutation: &MessageProjectionMutation,
) -> Result<(), MessageProjectionStoreFailure> {
    let encoded = EncodedMutation::from_mutation(mutation)?;
    let sequence = next_sequence(transaction, &encoded.room_id).await?;
    if insert_event(transaction, &encoded, sequence).await? == 0 {
        return Ok(());
    }
    match mutation {
        MessageProjectionMutation::Preview(_) => {
            if insert_current(transaction, &encoded, sequence).await? == 1 {
                apply_pending_revisions(transaction, &encoded.room_id, &encoded.message_id).await?;
            }
        }
        MessageProjectionMutation::Revision(_) => {
            apply_revision(transaction, &encoded, sequence).await?;
        }
    }
    Ok(())
}

async fn next_sequence(
    transaction: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> Result<i64, MessageProjectionStoreFailure> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sequence), 0) + 1
         FROM message_projection_event
         WHERE room_id = ?",
    )
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
            transaction_id, actor_agent_id, actor_json, preview_json,
            content_json, relation_target_message_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(&event.actor_agent_id)
    .bind(&event.actor_json)
    .bind(event.preview_json.as_deref())
    .bind(event.content_json.as_deref())
    .bind(event.relation_target_message_id.as_deref())
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
    sqlx::query(
        "INSERT INTO message_current_projection (
            message_id, room_id, base_event_id, first_sequence, last_sequence,
            created_at_unix_ms, origin_server_timestamp, actor_agent_id,
            actor_json, preview_json, content_json, relation_target_message_id,
            visibility, last_revision_event_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', NULL)
         ON CONFLICT(message_id) DO NOTHING",
    )
    .bind(&event.message_id)
    .bind(&event.room_id)
    .bind(&event.event_id)
    .bind(sequence)
    .bind(sequence)
    .bind(event.created_at_unix_ms)
    .bind(event.origin_server_timestamp)
    .bind(&event.actor_agent_id)
    .bind(&event.actor_json)
    .bind(preview_json)
    .bind(content_json)
    .bind(event.relation_target_message_id.as_deref())
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
        "SELECT event_id, sequence, revision_kind, actor_agent_id,
                preview_json, content_json
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
                actor_agent_id: row
                    .try_get("actor_agent_id")
                    .map_err(|_| corrupt_projection_failure())?,
                preview_json: row
                    .try_get("preview_json")
                    .map_err(|_| corrupt_projection_failure())?,
                content_json: row
                    .try_get("content_json")
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
            actor_agent_id: event.actor_agent_id.clone(),
            preview_json: event.preview_json.clone(),
            content_json: event.content_json.clone(),
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
    actor_agent_id: String,
    preview_json: Option<String>,
    content_json: Option<String>,
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
    sqlx::query(
        "UPDATE message_current_projection
         SET preview_json = ?, content_json = ?, last_revision_event_id = ?,
             last_sequence = MAX(last_sequence, ?)
         WHERE room_id = ? AND message_id = ? AND actor_agent_id = ?
           AND visibility = 'active'",
    )
    .bind(preview_json)
    .bind(content_json)
    .bind(&revision.event_id)
    .bind(revision.sequence)
    .bind(revision.room_id)
    .bind(revision.message_id)
    .bind(&revision.actor_agent_id)
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
         SET content_json = NULL, visibility = 'redacted',
             last_revision_event_id = ?, last_sequence = MAX(last_sequence, ?)
         WHERE room_id = ? AND message_id = ? AND actor_agent_id = ?
           AND visibility = 'active'",
    )
    .bind(&revision.event_id)
    .bind(revision.sequence)
    .bind(revision.room_id)
    .bind(revision.message_id)
    .bind(&revision.actor_agent_id)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| map_sqlx_error(&error))
}

async fn persist_issues(
    transaction: &mut Transaction<'_, Sqlite>,
    batch: &MessageProjectionBatch,
) -> Result<(), MessageProjectionStoreFailure> {
    for issue in batch.issues() {
        sqlx::query(
            "INSERT OR IGNORE INTO message_sync_issue
             (sync_token, room_id, event_id, reason)
             VALUES (?, ?, ?, ?)",
        )
        .bind(batch.next_batch().as_str())
        .bind(issue.room_id.as_str())
        .bind(
            issue
                .event_id
                .as_ref()
                .map_or("", |event_id| event_id.as_str()),
        )
        .bind(issue.reason.as_str())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(&error))?;
    }
    Ok(())
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
    actor_agent_id: String,
    actor_json: String,
    preview_json: Option<String>,
    content_json: Option<String>,
    relation_target_message_id: Option<String>,
}

impl EncodedMutation {
    fn from_mutation(
        mutation: &MessageProjectionMutation,
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
                actor_agent_id: preview.actor.identity().agent_id().to_string(),
                actor_json: encode_actor(&preview.actor),
                preview_json: Some(encode_preview(&preview.preview)),
                content_json: Some(encode_content(preview.content)),
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
                actor_agent_id: revision.actor.identity().agent_id().to_string(),
                actor_json: encode_actor(&revision.actor),
                preview_json: revision.preview.as_ref().map(encode_preview),
                content_json: revision.content.map(encode_content),
                relation_target_message_id: None,
            }),
        }
    }
}

fn encode_actor(actor: &ProjectedMessageActor) -> String {
    let identity = actor.identity();
    json!({
        "agentId": identity.agent_id().to_string(),
        "instanceId": identity.agent_instance_id().to_string(),
        "displayName": identity.display_name(),
        "matrixUserId": identity.matrix_user_id().as_str(),
        "avatarUrl": identity.avatar_url(),
        "provenance": actor.provenance().as_str()
    })
    .to_string()
}

fn encode_preview(preview: &MessagePreview) -> String {
    json!({
        "title": preview.title().as_str(),
        "summary": preview.summary().as_str(),
        "contentType": preview.content_type().as_str(),
        "language": preview.language().map(agent_room_domain::messages::MessageLanguage::as_str),
        "sensitivity": preview.sensitivity().as_str(),
        "riskFlags": preview
            .risk_flags()
            .iter()
            .map(agent_room_domain::messages::MessageRiskFlag::as_str)
            .collect::<Vec<_>>()
    })
    .to_string()
}

fn encode_content(content: MessageContentReference) -> String {
    json!({
        "contentId": content.content_id().to_string(),
        "digestSha256": encode_hex(content.digest().as_bytes()),
        "sizeBytes": content.size_bytes(),
        "fetchMode": "on_demand"
    })
    .to_string()
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
