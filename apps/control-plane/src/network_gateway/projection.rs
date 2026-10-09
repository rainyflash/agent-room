//! 把本机 Bridge 的同步与验签接到网关上：投影不落库，只截下这一批的结果，再转成收件箱里的变化。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_application::ports::{
    AgentInstanceVerificationRecord, AgentInstanceVerificationRepository, MatrixEventId,
    MatrixRoomId, MatrixSyncToken, NetworkAgentInboxChange, NetworkAgentInboxMessage,
    NetworkAgentMessageActor, NetworkAgentRoomRecord, PortFuture,
};
use agent_room_bridge_core::{
    agent_verification::{
        AgentInstanceVerificationGateway, AgentInstanceVerificationGatewayFailure,
        AgentInstanceVerificationGatewayFailureKind, AgentInstanceVerificationGatewayResult,
    },
    messages::{
        MessageBackfillBatch, MessageProjectionBatch, MessageProjectionMutation,
        MessageProjectionStoreFailure, MessageTimelineProjectionStore, PendingTimelineGap,
        ProjectedMessagePreview, ProjectedMessageRevision,
    },
};
use agent_room_bridge_ipc::{
    IpcMessagePreviewSummary,
    previews::{
        PreviewRoom, PreviewText, PreviewViewer, actor_user_and_name, attach_stored_reply,
        preview_for, preview_summary, sent_at_ms,
    },
};
use agent_room_domain::{
    ids::{AgentInstanceId, MessageId},
    messages::{MessageRelation, MessageRevisionKind},
    time::UtcMillis,
};
use serde_json::{Map, Value};

/// 替换修订能改的字段；消息 ID、作者、时间、回复对象都不变。
const REPLACEABLE_FIELDS: [&str; 7] = [
    "conversation",
    "title",
    "summary",
    "content",
    "language",
    "sensitivity",
    "riskFlags",
];

/// 只记下 `apply` 收到的那一批。网络 Agent 不补缺口：一次来得太多时只收最近的。
#[derive(Default)]
pub(crate) struct CapturedProjection {
    batch: Mutex<Option<MessageProjectionBatch>>,
}

impl CapturedProjection {
    pub(crate) fn take(&self) -> Option<MessageProjectionBatch> {
        self.batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl MessageTimelineProjectionStore for CapturedProjection {
    fn apply<'a>(
        &'a self,
        batch: &'a MessageProjectionBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        *self.batch.lock().unwrap_or_else(PoisonError::into_inner) = Some(batch.clone());
        Box::pin(async { Ok(()) })
    }

    fn sync_cursor(
        &self,
    ) -> PortFuture<'_, Result<Option<MatrixSyncToken>, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(None) })
    }

    fn room_has_messages<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<bool, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(false) })
    }

    fn known_events<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event_ids: &'a [MatrixEventId],
    ) -> PortFuture<'a, Result<Vec<MatrixEventId>, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn pending_gaps(
        &self,
        _limit: u16,
    ) -> PortFuture<'_, Result<Vec<PendingTimelineGap>, MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn apply_backfill<'a>(
        &'a self,
        _batch: &'a MessageBackfillBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>> {
        Box::pin(async { Ok(()) })
    }
}

/// 验签查库：与本机 Bridge 经控制面查的是同一份实例记录，只是在进程内直接读。
pub(super) struct InstanceVerification {
    repository: Arc<dyn AgentInstanceVerificationRepository>,
}

impl InstanceVerification {
    pub(super) fn new(repository: Arc<dyn AgentInstanceVerificationRepository>) -> Self {
        Self { repository }
    }
}

impl AgentInstanceVerificationGateway for InstanceVerification {
    fn resolve(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>>
    {
        Box::pin(async move {
            match self.repository.find_verification_record(instance_id).await {
                Ok(Some(record)) => Ok(record),
                Ok(None) => Err(AgentInstanceVerificationGatewayFailure::new(
                    AgentInstanceVerificationGatewayFailureKind::NotFound,
                )),
                Err(_) => Err(AgentInstanceVerificationGatewayFailure::new(
                    AgentInstanceVerificationGatewayFailureKind::Unavailable,
                )),
            }
        })
    }
}

/// 消息记录里找来的被回复的那几条，按房间和消息 ID 找。
pub(super) type StoredReplies = HashMap<(MatrixRoomId, MessageId), IpcMessagePreviewSummary>;

/// 这一批里回复的、却不在这一批里的那几条：要去消息记录里找。
pub(super) fn reply_targets(batch: &MessageProjectionBatch) -> Vec<(MatrixRoomId, MessageId)> {
    let in_batch = batch_previews(batch);
    let mut targets: Vec<(MatrixRoomId, MessageId)> = batch
        .mutations()
        .iter()
        .filter_map(|mutation| match mutation {
            MessageProjectionMutation::Preview(preview) => match preview.relation? {
                MessageRelation::ReplyTo(id) => {
                    (!in_batch.contains_key(&id)).then(|| (preview.room_id.clone(), id))
                }
            },
            MessageProjectionMutation::Revision(_) => None,
        })
        .collect();
    targets.sort_by(|left, right| {
        (left.0.as_str(), left.1.as_uuid()).cmp(&(right.0.as_str(), right.1.as_uuid()))
    });
    targets.dedup();
    targets
}

fn batch_previews(batch: &MessageProjectionBatch) -> HashMap<MessageId, &ProjectedMessagePreview> {
    batch
        .mutations()
        .iter()
        .filter_map(|mutation| match mutation {
            MessageProjectionMutation::Preview(preview) => Some((preview.message_id, preview)),
            MessageProjectionMutation::Revision(_) => None,
        })
        .collect()
}

/// 验签通过的变化按时间线顺序转成收件箱里的变化。Agent 自己发的只记进消息记录，不进自己的
/// 收件箱；治理隐藏与本机 Bridge 一样先不处理。
///
/// 存的都是全文；一批交给 Agent 时再截断长正文，按 ID 取还能给全文。被回复的那条先在同一批里
/// 找，找不到再用消息记录里的（`stored`，见 specs/agent-reading/design.md 第 5 步）。
pub(super) fn inbox_changes(
    batch: Option<MessageProjectionBatch>,
    viewer: PreviewViewer<'_>,
    rooms: &[NetworkAgentRoomRecord],
    stored: &StoredReplies,
) -> Vec<NetworkAgentInboxChange> {
    let Some(batch) = batch else {
        return Vec::new();
    };
    let in_batch = batch_previews(&batch);
    batch
        .mutations()
        .iter()
        .filter_map(|mutation| match mutation {
            MessageProjectionMutation::Preview(preview) => {
                let target = preview.relation.map(|relation| match relation {
                    MessageRelation::ReplyTo(id) => id,
                });
                let mut summary = preview_for(
                    preview,
                    viewer,
                    preview_room(rooms, &preview.room_id),
                    target.and_then(|id| in_batch.get(&id).copied()),
                    PreviewText::Full,
                );
                if summary.reply_to.is_none()
                    && let Some(replied) =
                        target.and_then(|id| stored.get(&(preview.room_id.clone(), id)))
                {
                    attach_stored_reply(&mut summary, replied);
                }
                inbox_message(preview, summary)
            }
            MessageProjectionMutation::Revision(revision) => match revision.kind {
                MessageRevisionKind::Replace => Some(NetworkAgentInboxChange::Replace {
                    room_id: revision.room_id.clone(),
                    message_id: revision.target_message_id,
                    actor_key: revision.actor.subject_key(),
                    patch: replacement_patch(revision)?,
                }),
                MessageRevisionKind::Redact => Some(NetworkAgentInboxChange::Redact {
                    room_id: revision.room_id.clone(),
                    message_id: revision.target_message_id,
                    actor_key: revision.actor.subject_key(),
                }),
                MessageRevisionKind::Moderate => None,
            },
        })
        .collect()
}

fn inbox_message(
    preview: &ProjectedMessagePreview,
    summary: IpcMessagePreviewSummary,
) -> Option<NetworkAgentInboxChange> {
    let (matrix_user_id, name) = actor_user_and_name(&summary.actor);
    Some(NetworkAgentInboxChange::Message(NetworkAgentInboxMessage {
        event_id: preview.event_id.clone(),
        room_id: preview.room_id.clone(),
        message_id: preview.message_id,
        actor_key: preview.actor.subject_key(),
        actor: NetworkAgentMessageActor {
            matrix_user_id: matrix_user_id.to_owned(),
            name_folded: name.to_lowercase(),
        },
        from_me: summary.from_me,
        mentions_me: summary.mentions_me,
        preview: serde_json::to_value(summary).ok()?,
        sent_at: UtcMillis::new(sent_at_ms(preview)).unwrap_or(preview.created_at),
        // 补不回来的一段在存之前由往回补那一步标上（`backfill::mark_losses`）。
        gap: None,
    }))
}

/// 这条消息所在的房间：房间名和这个网络 Agent 什么时候进来的。不在记录里的房间都不知道。
fn preview_room<'a>(
    rooms: &'a [NetworkAgentRoomRecord],
    room_id: &MatrixRoomId,
) -> PreviewRoom<'a> {
    rooms
        .iter()
        .find(|room| room.matrix_room_id.as_str() == room_id.as_str())
        .map_or_else(PreviewRoom::default, |room| PreviewRoom {
            name: room.name.as_deref(),
            joined_at_ms: Some(room.joined_at.value()),
        })
}

/// 替换修订带来的新预览，只留能改的字段，合并进收件箱里原来那条。
fn replacement_patch(revision: &ProjectedMessageRevision) -> Option<Value> {
    let replaced = ProjectedMessagePreview {
        event_id: revision.event_id.clone(),
        transaction_id: None,
        room_id: revision.room_id.clone(),
        message_id: revision.target_message_id,
        created_at: revision.created_at,
        origin_server_timestamp: revision.origin_server_timestamp,
        actor: revision.actor.clone(),
        preview: revision.preview.clone()?,
        content: revision.content.clone()?,
        relation: None,
    };
    let Value::Object(fields) = serde_json::to_value(preview_summary(&replaced)).ok()? else {
        return None;
    };
    let patch: Map<String, Value> = fields
        .into_iter()
        .filter(|(key, _)| REPLACEABLE_FIELDS.contains(&key.as_str()))
        .collect();
    Some(Value::Object(patch))
}
