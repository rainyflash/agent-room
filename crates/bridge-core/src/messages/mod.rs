mod incoming;
mod model;
mod outgoing;
mod ports;
mod projection;
mod wire;

pub use incoming::{
    MessageAuthenticationDecision, MessageAuthenticationFailure, MessageAuthenticationFailureKind,
    MessageEventAuthenticator, MessageSyncDependencies, MessageSyncFailure, MessageSyncFailureKind,
    MessageSyncOutcome, MessageSyncService,
};
pub use model::{
    EditMessageRequest, MessageBody, MessageRequestError, RedactMessageRequest, SendMessageRequest,
};
pub use outgoing::{
    MatrixMessageEventPublisher, MessagePublicationDependencies, MessagePublicationFailure,
    MessagePublicationFailureKind, MessagePublicationOutcome, MessagePublicationService,
};
pub use ports::{
    MessageContentBindRequest, MessageContentFailure, MessageContentFailureKind,
    MessageContentGateway, MessageContentRecord, MessageContentRedactRequest,
    MessageContentUploadRequest, MessageEventPublisher, MessageStoreFailure,
    MessageStoreFailureKind, MessageSubmissionClaim, MessageSubmissionClaimOutcome,
    MessageSubmissionFingerprint, MessageSubmissionKind, MessageSubmissionRecord,
    MessageSubmissionRepository, MessageSubmissionState,
};
pub use projection::{
    MessageProjectionBatch, MessageProjectionMutation, MessageProjectionStoreFailure,
    MessageProjectionStoreFailureKind, MessageSyncIssue, MessageSyncIssueReason,
    MessageTimelineGap, MessageTimelineProjectionStore, ProjectedActorInstanceVerification,
    ProjectedMessageActor, ProjectedMessagePreview, ProjectedMessageRevision,
};
