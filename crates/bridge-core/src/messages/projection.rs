use agent_room_application::ports::{
    MatrixBackfillToken, MatrixEventId, MatrixRoomId, MatrixSyncToken, MatrixTransactionId,
    PortFuture,
};
use agent_room_domain::{
    ids::{MessageId, MessageRevisionId},
    messages::{
        MessageContentReference, MessagePreview, MessageProvenance, MessageRelation,
        MessageRevisionKind,
    },
    time::UtcMillis,
};

use crate::agent_identity::BridgeAgentIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedMessageActor {
    identity: BridgeAgentIdentity,
    provenance: MessageProvenance,
    instance_verification: ProjectedActorInstanceVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectedActorInstanceVerification {
    Active,
    RevokedAfterEvent,
}

impl ProjectedActorInstanceVerification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::RevokedAfterEvent => "revoked_after_event",
        }
    }
}

impl ProjectedMessageActor {
    pub const fn new(identity: BridgeAgentIdentity, provenance: MessageProvenance) -> Self {
        Self {
            identity,
            provenance,
            instance_verification: ProjectedActorInstanceVerification::Active,
        }
    }

    #[must_use]
    pub const fn with_instance_verification(
        mut self,
        instance_verification: ProjectedActorInstanceVerification,
    ) -> Self {
        self.instance_verification = instance_verification;
        self
    }

    pub const fn identity(&self) -> &BridgeAgentIdentity {
        &self.identity
    }

    pub const fn provenance(&self) -> MessageProvenance {
        self.provenance
    }

    pub const fn instance_verification(&self) -> ProjectedActorInstanceVerification {
        self.instance_verification
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
        actor.instance_verification = ProjectedActorInstanceVerification::RevokedAfterEvent;
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
