use std::path::Path;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::handoffs::{
    TargetedHandoffInbox, TargetedHandoffInboxFailure, TargetedHandoffInboxFailureKind,
    TargetedHandoffInboxRecordOutcome, TargetedHandoffTarget,
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        HandoffContentReference, HandoffPermission, HandoffPermissions, HandoffPurpose,
        HandoffSourceEventId, TargetedHandoff, TargetedHandoffFields, TargetedHandoffStatus,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::{Uuid, Version};

use crate::{
    database::{SqliteBridgeStorageOpenFailure, open_handoff_pool},
    error::{SqliteFailureKind, classify},
};

#[derive(Clone)]
pub struct SqliteTargetedHandoffInbox {
    pool: SqlitePool,
}

impl SqliteTargetedHandoffInbox {
    /// 打开并迁移只含云端交接元数据的本机收件箱。
    ///
    /// # Errors
    ///
    /// 目录不可创建、数据库不可连接或迁移失败时返回错误。
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SqliteBridgeStorageOpenFailure> {
        open_handoff_pool(path.as_ref())
            .await
            .map(|pool| Self { pool })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    async fn accept_internal(
        &self,
        handoff: &TargetedHandoff,
    ) -> Result<TargetedHandoffInboxRecordOutcome, TargetedHandoffInboxFailure> {
        if handoff.status() != TargetedHandoffStatus::Delivered {
            return Err(failure(TargetedHandoffInboxFailureKind::Conflict));
        }
        let fields = handoff.fields();
        let delivered_at = handoff
            .delivered_at()
            .ok_or_else(|| failure(TargetedHandoffInboxFailureKind::Corrupt))?;
        let permissions = serde_json::to_string(
            &fields
                .permissions
                .iter()
                .map(HandoffPermission::as_str)
                .collect::<Vec<_>>(),
        )
        .map_err(|_| failure(TargetedHandoffInboxFailureKind::Corrupt))?;
        let inserted = sqlx::query(
            "INSERT INTO targeted_handoff_inbox (
                handoff_id, principal_id, source_room_id, source_event_id, source_message_id,
                target_agent_id, target_instance_id, content_id, content_digest,
                content_byte_length, content_media_type, permissions_json, purpose,
                created_at_unix_ms, queued_at_unix_ms, delivered_at_unix_ms,
                expires_at_unix_ms, version
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(handoff_id) DO NOTHING",
        )
        .bind(fields.id.to_string())
        .bind(fields.principal_id.to_string())
        .bind(fields.source_room_id.as_str())
        .bind(fields.source_event_id.as_str())
        .bind(fields.source_message_id.to_string())
        .bind(fields.target_agent_id.to_string())
        .bind(fields.target_instance_id.to_string())
        .bind(fields.content.content_id().to_string())
        .bind(fields.content.digest().as_bytes().as_slice())
        .bind(i64::try_from(fields.content.byte_length().value()).map_err(|_| corrupt())?)
        .bind(fields.content.media_type().as_str())
        .bind(permissions)
        .bind(fields.purpose.as_str())
        .bind(fields.created_at.value())
        .bind(handoff.queued_at().value())
        .bind(delivered_at.value())
        .bind(fields.expires_at.value())
        .bind(i64::try_from(handoff.version()).map_err(|_| corrupt())?)
        .execute(&self.pool)
        .await
        .map_err(|error| map_sqlx_error(&error))?
        .rows_affected();
        let existing = self
            .find_internal(
                TargetedHandoffTarget {
                    agent_id: fields.target_agent_id,
                    instance_id: fields.target_instance_id,
                },
                fields.id,
            )
            .await?
            .ok_or_else(corrupt)?;
        if &existing != handoff {
            return Err(failure(TargetedHandoffInboxFailureKind::Conflict));
        }
        Ok(if inserted == 1 {
            TargetedHandoffInboxRecordOutcome::Created(existing)
        } else {
            TargetedHandoffInboxRecordOutcome::Existing(existing)
        })
    }

    async fn find_internal(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> Result<Option<TargetedHandoff>, TargetedHandoffInboxFailure> {
        sqlx::query(FIND_QUERY)
            .bind(handoff_id.to_string())
            .bind(target.agent_id.to_string())
            .bind(target.instance_id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(&error))?
            .as_ref()
            .map(decode_handoff)
            .transpose()
    }

    async fn list_internal(
        &self,
        target: TargetedHandoffTarget,
        limit: u16,
    ) -> Result<Vec<TargetedHandoff>, TargetedHandoffInboxFailure> {
        if limit == 0 || limit > 100 {
            return Err(failure(TargetedHandoffInboxFailureKind::Conflict));
        }
        sqlx::query(LIST_QUERY)
            .bind(target.agent_id.to_string())
            .bind(target.instance_id.to_string())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(&error))?
            .iter()
            .map(decode_handoff)
            .collect()
    }

    async fn remove_internal(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> Result<bool, TargetedHandoffInboxFailure> {
        sqlx::query(
            "DELETE FROM targeted_handoff_inbox
              WHERE handoff_id = ? AND target_agent_id = ? AND target_instance_id = ?",
        )
        .bind(handoff_id.to_string())
        .bind(target.agent_id.to_string())
        .bind(target.instance_id.to_string())
        .execute(&self.pool)
        .await
        .map(|result| result.rows_affected() == 1)
        .map_err(|error| map_sqlx_error(&error))
    }
}

impl TargetedHandoffInbox for SqliteTargetedHandoffInbox {
    fn accept<'a>(
        &'a self,
        handoff: &'a TargetedHandoff,
    ) -> PortFuture<'a, Result<TargetedHandoffInboxRecordOutcome, TargetedHandoffInboxFailure>>
    {
        Box::pin(self.accept_internal(handoff))
    }

    fn find(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<TargetedHandoff>, TargetedHandoffInboxFailure>> {
        Box::pin(self.find_internal(target, handoff_id))
    }

    fn list(
        &self,
        target: TargetedHandoffTarget,
        limit: u16,
    ) -> PortFuture<'_, Result<Vec<TargetedHandoff>, TargetedHandoffInboxFailure>> {
        Box::pin(self.list_internal(target, limit))
    }

    fn remove(
        &self,
        target: TargetedHandoffTarget,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<bool, TargetedHandoffInboxFailure>> {
        Box::pin(self.remove_internal(target, handoff_id))
    }

    fn remove_expired(
        &self,
        target: TargetedHandoffTarget,
        observed_at: UtcMillis,
    ) -> PortFuture<'_, Result<u64, TargetedHandoffInboxFailure>> {
        Box::pin(async move {
            sqlx::query(
                "DELETE FROM targeted_handoff_inbox
                  WHERE target_agent_id = ? AND target_instance_id = ?
                    AND expires_at_unix_ms <= ?",
            )
            .bind(target.agent_id.to_string())
            .bind(target.instance_id.to_string())
            .bind(observed_at.value())
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(|error| map_sqlx_error(&error))
        })
    }
}

const FIND_QUERY: &str =
    "SELECT handoff_id, principal_id, source_room_id, source_event_id, source_message_id,
            target_agent_id, target_instance_id, content_id, content_digest,
            content_byte_length, content_media_type, permissions_json, purpose,
            created_at_unix_ms, queued_at_unix_ms, delivered_at_unix_ms,
            expires_at_unix_ms, version
       FROM targeted_handoff_inbox
      WHERE handoff_id = ? AND target_agent_id = ? AND target_instance_id = ?";

const LIST_QUERY: &str =
    "SELECT handoff_id, principal_id, source_room_id, source_event_id, source_message_id,
            target_agent_id, target_instance_id, content_id, content_digest,
            content_byte_length, content_media_type, permissions_json, purpose,
            created_at_unix_ms, queued_at_unix_ms, delivered_at_unix_ms,
            expires_at_unix_ms, version
       FROM targeted_handoff_inbox
      WHERE target_agent_id = ? AND target_instance_id = ?
      ORDER BY delivered_at_unix_ms, handoff_id
      LIMIT ?";

fn decode_handoff(row: &SqliteRow) -> Result<TargetedHandoff, TargetedHandoffInboxFailure> {
    let digest = row
        .try_get::<Vec<u8>, _>("content_digest")
        .map_err(|_| corrupt())?
        .try_into()
        .map_err(|_| corrupt())?;
    let permissions = serde_json::from_str::<Vec<String>>(
        &row.try_get::<String, _>("permissions_json")
            .map_err(|_| corrupt())?,
    )
    .map_err(|_| corrupt())?
    .iter()
    .map(|value| HandoffPermission::try_from(value.as_str()).map_err(|_| corrupt()))
    .collect::<Result<Vec<_>, _>>()?;
    let byte_length = row
        .try_get::<i64, _>("content_byte_length")
        .map_err(|_| corrupt())?;
    let version = row.try_get::<i64, _>("version").map_err(|_| corrupt())?;
    let fields = TargetedHandoffFields {
        id: HandoffId::from_uuid(parse_v7(row, "handoff_id")?),
        principal_id: PrincipalId::from_uuid(parse_v7(row, "principal_id")?),
        source_room_id: MatrixRoomReference::new(
            row.try_get::<String, _>("source_room_id")
                .map_err(|_| corrupt())?,
        )
        .map_err(|_| corrupt())?,
        source_event_id: HandoffSourceEventId::new(
            row.try_get::<String, _>("source_event_id")
                .map_err(|_| corrupt())?,
        )
        .map_err(|_| corrupt())?,
        source_message_id: MessageId::from_uuid(parse_v7(row, "source_message_id")?),
        target_agent_id: AgentId::from_uuid(parse_v7(row, "target_agent_id")?),
        target_instance_id: AgentInstanceId::from_uuid(parse_v7(row, "target_instance_id")?),
        content: HandoffContentReference::new(
            ContentId::from_uuid(parse_v7(row, "content_id")?),
            Sha256Digest::from_bytes(digest),
            ContentByteLength::new(u64::try_from(byte_length).map_err(|_| corrupt())?)
                .map_err(|_| corrupt())?,
            ContentMediaType::new(
                row.try_get::<String, _>("content_media_type")
                    .map_err(|_| corrupt())?,
            )
            .map_err(|_| corrupt())?,
        ),
        permissions: HandoffPermissions::new(permissions).map_err(|_| corrupt())?,
        purpose: HandoffPurpose::try_from(
            row.try_get::<String, _>("purpose")
                .map_err(|_| corrupt())?
                .as_str(),
        )
        .map_err(|_| corrupt())?,
        created_at: decode_time(row, "created_at_unix_ms")?,
        expires_at: decode_time(row, "expires_at_unix_ms")?,
    };
    TargetedHandoff::restore(
        fields,
        TargetedHandoffStatus::Delivered,
        decode_time(row, "queued_at_unix_ms")?,
        Some(decode_time(row, "delivered_at_unix_ms")?),
        None,
        None,
        None,
        u64::try_from(version).map_err(|_| corrupt())?,
    )
    .map_err(|_| corrupt())
}

fn parse_v7(row: &SqliteRow, column: &str) -> Result<Uuid, TargetedHandoffInboxFailure> {
    row.try_get::<String, _>(column)
        .ok()
        .and_then(|value| Uuid::parse_str(&value).ok())
        .filter(|value| value.get_version() == Some(Version::SortRand))
        .ok_or_else(corrupt)
}

fn decode_time(row: &SqliteRow, column: &str) -> Result<UtcMillis, TargetedHandoffInboxFailure> {
    UtcMillis::new(row.try_get(column).map_err(|_| corrupt())?).map_err(|_| corrupt())
}

fn map_sqlx_error(error: &sqlx::Error) -> TargetedHandoffInboxFailure {
    failure(match classify(error) {
        SqliteFailureKind::Conflict => TargetedHandoffInboxFailureKind::Conflict,
        SqliteFailureKind::NotFound => TargetedHandoffInboxFailureKind::NotFound,
        SqliteFailureKind::Corrupt => TargetedHandoffInboxFailureKind::Corrupt,
        SqliteFailureKind::Unavailable => TargetedHandoffInboxFailureKind::Unavailable,
    })
}

const fn corrupt() -> TargetedHandoffInboxFailure {
    failure(TargetedHandoffInboxFailureKind::Corrupt)
}

const fn failure(kind: TargetedHandoffInboxFailureKind) -> TargetedHandoffInboxFailure {
    TargetedHandoffInboxFailure::new(kind)
}
