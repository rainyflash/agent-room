//! 把本机 Bridge 的同步与验签接到网关上：投影不落库，只截下这一批的结果，再转成收件箱里的变化。

use std::sync::{Arc, Mutex, PoisonError};

use agent_room_application::ports::{
    AgentInstanceVerificationRecord, AgentInstanceVerificationRepository, MatrixEventId,
    MatrixRoomId, MatrixSyncToken, NetworkAgentInboxChange, NetworkAgentInboxMessage, PortFuture,
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
use agent_room_bridge_ipc::previews::preview_summary;
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId},
    messages::MessageRevisionKind,
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
pub(super) struct CapturedProjection {
    batch: Mutex<Option<MessageProjectionBatch>>,
}

impl CapturedProjection {
    pub(super) fn take(&self) -> Option<MessageProjectionBatch> {
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

/// 验签通过的变化按时间线顺序转成收件箱里的变化。Agent 自己发的不进自己的收件箱；
/// 治理隐藏与本机 Bridge 一样先不处理。
pub(super) fn inbox_changes(
    batch: Option<MessageProjectionBatch>,
    own_agent: AgentId,
) -> Vec<NetworkAgentInboxChange> {
    let Some(batch) = batch else {
        return Vec::new();
    };
    batch
        .mutations()
        .iter()
        .filter_map(|mutation| match mutation {
            MessageProjectionMutation::Preview(preview) => {
                if preview
                    .actor
                    .agent_identity()
                    .is_some_and(|identity| identity.agent_id() == own_agent)
                {
                    return None;
                }
                Some(NetworkAgentInboxChange::Message(NetworkAgentInboxMessage {
                    event_id: preview.event_id.clone(),
                    room_id: preview.room_id.clone(),
                    message_id: preview.message_id,
                    actor_key: preview.actor.subject_key(),
                    preview: serde_json::to_value(preview_summary(preview)).ok()?,
                }))
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
