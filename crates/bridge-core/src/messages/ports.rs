use std::sync::Arc;

use agent_room_application::ports::{
    MatrixAcceptedEvent, MatrixEvent, MatrixEventId, MatrixResult, MatrixRoomId,
    MatrixTransactionId, PortFuture,
};
use agent_room_domain::{
    content::{ContentByteLength, ContentEncryptionMode, ContentMediaType, Sha256Digest},
    ids::{ContentId, ContentUploadRequestId, MessageSubmissionId},
    time::UtcMillis,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageContentFailureKind {
    InvalidRequest,
    Denied,
    Conflict,
    Unavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageContentFailure {
    kind: MessageContentFailureKind,
}

impl MessageContentFailure {
    pub const fn new(kind: MessageContentFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> MessageContentFailureKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContentUploadRequest {
    pub request_id: ContentUploadRequestId,
    pub room_id: MatrixRoomId,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
    pub encryption_mode: ContentEncryptionMode,
    pub expires_at: Option<UtcMillis>,
    pub body: Arc<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContentRecord {
    pub content_id: ContentId,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContentBindRequest {
    pub content_id: ContentId,
    pub room_id: MatrixRoomId,
    pub event_id: MatrixEventId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageContentRedactRequest {
    pub content_id: ContentId,
}

pub trait MessageContentGateway: Send + Sync {
    fn upload<'a>(
        &'a self,
        request: &'a MessageContentUploadRequest,
    ) -> PortFuture<'a, Result<MessageContentRecord, MessageContentFailure>>;

    fn bind<'a>(
        &'a self,
        request: &'a MessageContentBindRequest,
    ) -> PortFuture<'a, Result<(), MessageContentFailure>>;

    fn redact<'a>(
        &'a self,
        request: &'a MessageContentRedactRequest,
    ) -> PortFuture<'a, Result<(), MessageContentFailure>>;
}

/// 将消息发送用例与完整 Matrix 会话能力隔离开的最小端口。
pub trait MessageEventPublisher: Send + Sync {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSubmissionKind {
    Preview,
    Replace,
    Redact,
}

impl MessageSubmissionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Replace => "replace",
            Self::Redact => "redact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSubmissionFingerprint([u8; 32]);

impl MessageSubmissionFingerprint {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSubmissionState {
    Claimed,
    SubmitUnknown,
    Accepted,
    Bound,
}

impl MessageSubmissionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::SubmitUnknown => "submit_unknown",
            Self::Accepted => "accepted",
            Self::Bound => "bound",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSubmissionClaim {
    pub submission_id: MessageSubmissionId,
    pub kind: MessageSubmissionKind,
    pub fingerprint: MessageSubmissionFingerprint,
    pub transaction_id: MatrixTransactionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSubmissionRecord {
    pub submission_id: MessageSubmissionId,
    pub kind: MessageSubmissionKind,
    pub fingerprint: MessageSubmissionFingerprint,
    pub transaction_id: MatrixTransactionId,
    pub state: MessageSubmissionState,
    pub event_id: Option<MatrixEventId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSubmissionClaimOutcome {
    Created(MessageSubmissionRecord),
    Existing(MessageSubmissionRecord),
}

impl MessageSubmissionClaimOutcome {
    pub const fn record(&self) -> &MessageSubmissionRecord {
        match self {
            Self::Created(record) | Self::Existing(record) => record,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStoreFailureKind {
    Conflict,
    NotFound,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageStoreFailure {
    kind: MessageStoreFailureKind,
}

impl MessageStoreFailure {
    pub const fn new(kind: MessageStoreFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> MessageStoreFailureKind {
        self.kind
    }
}

pub trait MessageSubmissionRepository: Send + Sync {
    fn claim<'a>(
        &'a self,
        claim: &'a MessageSubmissionClaim,
    ) -> PortFuture<'a, Result<MessageSubmissionClaimOutcome, MessageStoreFailure>>;

    fn mark_submit_unknown(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>>;

    fn mark_accepted<'a>(
        &'a self,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<MessageSubmissionRecord, MessageStoreFailure>>;

    fn mark_bound(
        &self,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, Result<MessageSubmissionRecord, MessageStoreFailure>>;

    fn observe_transaction<'a>(
        &'a self,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<Option<MessageSubmissionRecord>, MessageStoreFailure>>;
}
