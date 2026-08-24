mod content;
mod incoming;
mod model;
mod outgoing;
mod ports;
mod projection;
mod wire;

pub use content::{
    DownloadedMessageContent, MessageContentReadFailure, MessageContentReadFailureKind,
    MessageContentReadGateway, MessageContentReadRequest, OpenMessageContentDependencies,
    OpenMessageContentFailure, OpenMessageContentFailureKind, OpenMessageContentRequest,
    OpenMessageContentService, OpenedMessageContent,
};
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
    MessageContentSourceQuery, MessagePreviewPage, MessagePreviewQuery, MessagePreviewQueryError,
    MessageProjectionBatch, MessageProjectionMutation, MessageProjectionStoreFailure,
    MessageProjectionStoreFailureKind, MessageSyncIssue, MessageSyncIssueReason,
    MessageTimelineGap, MessageTimelineProjectionStore, MessageTimelineQueryFailure,
    MessageTimelineQueryFailureKind, MessageTimelineQueryRepository,
    ProjectedActorInstanceVerification, ProjectedMessageActor, ProjectedMessagePreview,
    ProjectedMessageRevision,
};
