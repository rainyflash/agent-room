use agent_room_application::ports::{
    MatrixBackfillToken, MatrixEventId, MatrixRoomId, MatrixSyncToken, MatrixTransactionId,
    PortFuture,
};
use agent_room_domain::{
    ids::{ContentId, MessageId, MessageRevisionId},
    messages::{
        MessageContentReference, MessagePreview, MessageProvenance, MessageRelation,
        MessageRevisionKind,
    },
    time::UtcMillis,
};

use crate::agent_identity::BridgeAgentIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedMessageActor {
    Agent {
        identity: BridgeAgentIdentity,
        provenance: MessageProvenance,
        instance_verification: ProjectedActorInstanceVerification,
    },
    Human {
        principal_id: agent_room_domain::ids::PrincipalId,
        display_name: String,
        matrix_user_id: agent_room_application::ports::MatrixUserId,
        avatar_url: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectedActorInstanceVerification {
    Active,
    RevokedAfterEvent,
    MatrixSenderMatched,
}

impl ProjectedActorInstanceVerification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::RevokedAfterEvent => "revoked_after_event",
            Self::MatrixSenderMatched => "matrix_sender_matched",
        }
    }
}

impl ProjectedMessageActor {
    pub const fn new(identity: BridgeAgentIdentity, provenance: MessageProvenance) -> Self {
        Self::Agent {
            identity,
            provenance,
            instance_verification: ProjectedActorInstanceVerification::Active,
        }
    }

    #[must_use]
    pub const fn with_instance_verification(
        mut self,
        verification: ProjectedActorInstanceVerification,
    ) -> Self {
        if let Self::Agent {
            instance_verification,
            ..
        } = &mut self
        {
            *instance_verification = verification;
        }
        self
    }

    pub const fn agent_identity(&self) -> Option<&BridgeAgentIdentity> {
        match self {
            Self::Agent { identity, .. } => Some(identity),
            Self::Human { .. } => None,
        }
    }

    pub const fn provenance(&self) -> MessageProvenance {
        match self {
            Self::Agent { provenance, .. } => *provenance,
            Self::Human { .. } => MessageProvenance::Human,
        }
    }

    pub const fn instance_verification(&self) -> ProjectedActorInstanceVerification {
        match self {
            Self::Agent {
                instance_verification,
                ..
            } => *instance_verification,
            Self::Human { .. } => ProjectedActorInstanceVerification::MatrixSenderMatched,
        }
    }

    /// 人类权限绑定到已经过 Matrix 校验的发送者，不能相信载荷自称的账号归属。
    pub fn subject_key(&self) -> String {
        match self {
            Self::Agent { identity, .. } => identity.agent_id().to_string(),
            Self::Human { matrix_user_id, .. } => format!("human:{}", matrix_user_id.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedMessagePreview {
    pub event_id: MatrixEventId,
    pub transaction_id: Option<MatrixTransactionId>,
    pub room_id: MatrixRoomId,
    pub message_id: MessageId,
    pub created_at: UtcMillis,
    pub origin_server_timestamp: Option<u64>,
    pub actor: ProjectedMessageActor,
    pub preview: MessagePreview,
    pub content: MessageContentReference,
    pub relation: Option<MessageRelation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedMessageRevision {
    pub event_id: MatrixEventId,
    pub transaction_id: Option<MatrixTransactionId>,
    pub room_id: MatrixRoomId,
    pub revision_id: MessageRevisionId,
    pub target_message_id: MessageId,
    pub created_at: UtcMillis,
    pub origin_server_timestamp: Option<u64>,
    pub actor: ProjectedMessageActor,
    pub kind: MessageRevisionKind,
    pub preview: Option<MessagePreview>,
    pub content: Option<MessageContentReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageProjectionMutation {
    Preview(ProjectedMessagePreview),
    Revision(ProjectedMessageRevision),
}

impl MessageProjectionMutation {
    pub const fn event_id(&self) -> &MatrixEventId {
        match self {
            Self::Preview(preview) => &preview.event_id,
            Self::Revision(revision) => &revision.event_id,
        }
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        match self {
            Self::Preview(preview) => &preview.room_id,
            Self::Revision(revision) => &revision.room_id,
        }
    }

    pub fn mark_instance_revoked_after_event(&mut self) {
        let actor = match self {
            Self::Preview(preview) => &mut preview.actor,
            Self::Revision(revision) => &mut revision.actor,
        };
        if let ProjectedMessageActor::Agent {
            instance_verification,
            ..
        } = actor
        {
            *instance_verification = ProjectedActorInstanceVerification::RevokedAfterEvent;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSyncIssueReason {
    MissingEnvelope,
    InvalidEnvelope,
    SenderMismatch,
    RoomMismatch,
    UnknownInstance,
    RevokedInstance,
    AgentInstanceMismatch,
    InvalidSignature,
    OutsideInstanceValidityWindow,
    UntrustedEncryptedSender,
}

impl MessageSyncIssueReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingEnvelope => "missing_envelope",
            Self::InvalidEnvelope => "invalid_envelope",
            Self::SenderMismatch => "sender_mismatch",
            Self::RoomMismatch => "room_mismatch",
            Self::UnknownInstance => "unknown_instance",
            Self::RevokedInstance => "revoked_instance",
            Self::AgentInstanceMismatch => "agent_instance_mismatch",
            Self::InvalidSignature => "invalid_signature",
            Self::OutsideInstanceValidityWindow => "outside_instance_validity_window",
            Self::UntrustedEncryptedSender => "untrusted_encrypted_sender",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSyncIssue {
    pub room_id: MatrixRoomId,
    pub event_id: Option<MatrixEventId>,
    pub reason: MessageSyncIssueReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTimelineGap {
    pub room_id: MatrixRoomId,
    pub previous_batch: Option<MatrixBackfillToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageProjectionBatch {
    next_batch: MatrixSyncToken,
    mutations: Vec<MessageProjectionMutation>,
    issues: Vec<MessageSyncIssue>,
    gaps: Vec<MessageTimelineGap>,
}

impl MessageProjectionBatch {
    pub const fn new(
        next_batch: MatrixSyncToken,
        mutations: Vec<MessageProjectionMutation>,
        issues: Vec<MessageSyncIssue>,
        gaps: Vec<MessageTimelineGap>,
    ) -> Self {
        Self {
            next_batch,
            mutations,
            issues,
            gaps,
        }
    }

    pub const fn next_batch(&self) -> &MatrixSyncToken {
        &self.next_batch
    }

    pub fn mutations(&self) -> &[MessageProjectionMutation] {
        &self.mutations
    }

    pub fn issues(&self) -> &[MessageSyncIssue] {
        &self.issues
    }

    pub fn gaps(&self) -> &[MessageTimelineGap] {
        &self.gaps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageProjectionStoreFailureKind {
    Unavailable,
    Conflict,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageProjectionStoreFailure {
    kind: MessageProjectionStoreFailureKind,
}

impl MessageProjectionStoreFailure {
    pub const fn new(kind: MessageProjectionStoreFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> MessageProjectionStoreFailureKind {
        self.kind
    }
}

pub trait MessageTimelineProjectionStore: Send + Sync {
    /// 按传入顺序原子应用事件、隔离记录、缺口标记和下一同步游标。
    fn apply<'a>(
        &'a self,
        batch: &'a MessageProjectionBatch,
    ) -> PortFuture<'a, Result<(), MessageProjectionStoreFailure>>;
}

const MAXIMUM_PREVIEW_PAGE_SIZE: u16 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePreviewQuery {
    after_event_id: Option<MatrixEventId>,
    room_id: MatrixRoomId,
    before_event_id: Option<MatrixEventId>,
    limit: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagePreviewQueryError {
    InvalidLimit,
}

impl MessagePreviewQuery {
    /// 创建有硬上限的房间预览分页查询。
    ///
    /// # Errors
    ///
    /// 分页大小为零或超过本地协议上限时返回错误。
    pub fn new(
        room_id: MatrixRoomId,
        before_event_id: Option<MatrixEventId>,
        limit: u16,
    ) -> Result<Self, MessagePreviewQueryError> {
        if limit == 0 || limit > MAXIMUM_PREVIEW_PAGE_SIZE {
            return Err(MessagePreviewQueryError::InvalidLimit);
        }
        Ok(Self {
            room_id,
            before_event_id,
            after_event_id: None,
            limit,
        })
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    /// 创建只包含指定事件之后内容的增量查询。
    ///
    /// # Errors
    ///
    /// 分页大小无效时返回错误。
    pub fn after(
        room_id: MatrixRoomId,
        cursor: MatrixEventId,
        limit: u16,
    ) -> Result<Self, MessagePreviewQueryError> {
        let mut query = Self::new(room_id, None, limit)?;
        query.after_event_id = Some(cursor);
        Ok(query)
    }

    pub const fn after_event_id(&self) -> Option<&MatrixEventId> {
        self.after_event_id.as_ref()
    }

    pub const fn before_event_id(&self) -> Option<&MatrixEventId> {
        self.before_event_id.as_ref()
    }

    pub const fn limit(&self) -> u16 {
        self.limit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePreviewPage {
    previews: Vec<ProjectedMessagePreview>,
    next_cursor: Option<MatrixEventId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContentSourceQuery {
    room_id: MatrixRoomId,
    content_id: ContentId,
}

impl MessageContentSourceQuery {
    pub const fn new(room_id: MatrixRoomId, content_id: ContentId) -> Self {
        Self {
            room_id,
            content_id,
        }
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }
}

impl MessagePreviewPage {
    pub const fn new(
        previews: Vec<ProjectedMessagePreview>,
        next_cursor: Option<MatrixEventId>,
    ) -> Self {
        Self {
            previews,
            next_cursor,
        }
    }

    pub fn previews(&self) -> &[ProjectedMessagePreview] {
        &self.previews
    }

    pub const fn next_cursor(&self) -> Option<&MatrixEventId> {
        self.next_cursor.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTimelineQueryFailureKind {
    Unavailable,
    CursorNotFound,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageTimelineQueryFailure {
    kind: MessageTimelineQueryFailureKind,
}

impl MessageTimelineQueryFailure {
    pub const fn new(kind: MessageTimelineQueryFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> MessageTimelineQueryFailureKind {
        self.kind
    }
}

pub trait MessageTimelineQueryRepository: Send + Sync {
    /// 从已验证的本地投影读取预览，不触发正文下载。
    fn list_previews<'a>(
        &'a self,
        query: &'a MessagePreviewQuery,
    ) -> PortFuture<'a, Result<MessagePreviewPage, MessageTimelineQueryFailure>>;

    /// 按正文标识读取已验证且仍有效的本地消息来源，不触发远端正文下载。
    fn find_content_source<'a>(
        &'a self,
        query: &'a MessageContentSourceQuery,
    ) -> PortFuture<'a, Result<Option<ProjectedMessagePreview>, MessageTimelineQueryFailure>>;
}

#[cfg(test)]
mod query_tests {
    use agent_room_application::ports::MatrixRoomId;

    use super::{MessagePreviewQuery, MessagePreviewQueryError};

    #[test]
    fn 预览分页在核心层保持硬上限() {
        let room_id = MatrixRoomId::new("!room:matrix.test").expect("房间标识有效");

        assert_eq!(
            MessagePreviewQuery::new(room_id.clone(), None, 0),
            Err(MessagePreviewQueryError::InvalidLimit)
        );
        assert_eq!(
            MessagePreviewQuery::new(room_id, None, 51),
            Err(MessagePreviewQueryError::InvalidLimit)
        );
    }
}
