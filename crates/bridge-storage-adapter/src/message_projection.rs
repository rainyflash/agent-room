use std::path::Path;

use agent_room_application::ports::{MatrixEventId, MatrixRoomId, PortFuture};
use agent_room_bridge_core::agent_identity::BridgeAgentIdentity;
use agent_room_bridge_core::messages::{
    MessagePreviewPage, MessagePreviewQuery, MessageProjectionBatch, MessageProjectionMutation,
    MessageProjectionStoreFailure, MessageProjectionStoreFailureKind,
    MessageTimelineProjectionStore, MessageTimelineQueryFailure, MessageTimelineQueryFailureKind,
    MessageTimelineQueryRepository, ProjectedActorInstanceVerification, ProjectedMessageActor,
    ProjectedMessagePreview,
};
use agent_room_domain::{
    content::{ContentMediaType, Sha256Digest},
    ids::{AgentId, AgentInstanceId, ContentId, MessageId},
    messages::{
        MessageContentReference, MessageLanguage, MessagePreview, MessageProvenance,
        MessageRelation, MessageRiskFlag, MessageRiskFlags, MessageSensitivity, MessageSummary,
        MessageTitle,
    },
    time::UtcMillis,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::{Row as _, Sqlite, SqlitePool, Transaction};
use uuid::{Uuid, Version};

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

impl MessageTimelineQueryRepository for SqliteMessageTimelineRepository {
    fn list_previews<'a>(
        &'a self,
        query: &'a MessagePreviewQuery,
    ) -> PortFuture<'a, Result<MessagePreviewPage, MessageTimelineQueryFailure>> {
        Box::pin(async move { self.query_previews(query).await })
    }
}

impl SqliteMessageTimelineRepository {
    async fn query_previews(
        &self,
        query: &MessagePreviewQuery,
    ) -> Result<MessagePreviewPage, MessageTimelineQueryFailure> {
        let cursor_sequence = match query.before_event_id() {
            Some(cursor) => Some(self.resolve_cursor(query.room_id(), cursor).await?),
            None => None,
        };
        let fetch_limit = i64::from(query.limit()) + 1;
        let rows = sqlx::query(
            "SELECT base_event_id, room_id, message_id, created_at_unix_ms,
                    origin_server_timestamp, actor_json, preview_json, content_json,
                    relation_target_message_id
             FROM message_current_projection
             WHERE room_id = ? AND visibility = 'active'
               AND (? IS NULL OR first_sequence < ?)
             ORDER BY first_sequence DESC
             LIMIT ?",
        )
        .bind(query.room_id().as_str())
        .bind(cursor_sequence)
        .bind(cursor_sequence)
        .bind(fetch_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| map_query_sqlx_error(&error))?;

        let has_more = rows.len() > usize::from(query.limit());
        let previews = rows
            .iter()
            .take(usize::from(query.limit()))
            .map(decode_preview_row)
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredActor {
    agent_id: String,
    instance_id: String,
    display_name: String,
    matrix_user_id: String,
    avatar_url: Option<String>,
    provenance: String,
    instance_verification: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPreview {
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
}

fn decode_preview_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ProjectedMessagePreview, MessageTimelineQueryFailure> {
    let actor = decode_actor(row.try_get("actor_json").map_err(|_| corrupt_query())?)?;
    let preview = decode_preview(row.try_get("preview_json").map_err(|_| corrupt_query())?)?;
    let content = decode_content(row.try_get("content_json").map_err(|_| corrupt_query())?)?;
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
        room_id: MatrixRoomId::new(
            row.try_get::<String, _>("room_id")
                .map_err(|_| corrupt_query())?,
        )
        .map_err(|_| corrupt_query())?,
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
    let mut identity = BridgeAgentIdentity::new(
        AgentId::from_uuid(parse_v7(&stored.agent_id)?),
        stored.display_name,
        stored.matrix_user_id,
        AgentInstanceId::from_uuid(parse_v7(&stored.instance_id)?),
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
    Ok(MessagePreview::new(
        MessageTitle::new(stored.title).map_err(|_| corrupt_query())?,
        MessageSummary::new(stored.summary).map_err(|_| corrupt_query())?,
        ContentMediaType::new(stored.content_type).map_err(|_| corrupt_query())?,
        language,
        MessageSensitivity::try_from(stored.sensitivity.as_str()).map_err(|_| corrupt_query())?,
        risk_flags,
    ))
}

fn decode_content(value: &str) -> Result<MessageContentReference, MessageTimelineQueryFailure> {
    let stored = serde_json::from_str::<StoredContent>(value).map_err(|_| corrupt_query())?;
    MessageContentReference::new(
        ContentId::from_uuid(parse_v7(&stored.content_id)?),
        Sha256Digest::from_bytes(decode_digest(&stored.digest_sha256)?),
        stored.size_bytes,
    )
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
        "provenance": actor.provenance().as_str(),
        "instanceVerification": actor.instance_verification().as_str()
    })
    .to_string()
}

fn encode_preview(preview: &MessagePreview) -> String {
    json!({
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
