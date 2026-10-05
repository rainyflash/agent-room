mod automation;
mod backfill;
mod chat;
pub use chat::validate_chat;
mod content;
mod crypto;
mod incoming;
mod model;
mod outgoing;
mod ports;
mod projection;
mod recovery;
mod room_state;
mod wire;

pub use room_state::{OwnMembership, RoomName, RoomStateChange, room_state_changes};

pub use automation::{
    AutomationAuthorizationDenial, AutomationAuthorizationFailure,
    AutomationAuthorizationFailureKind, AutomationAuthorizationGateway,
    AutomationAuthorizationRequest, AutomationAuthorizationResult,
};
pub use backfill::{MessageBackfillOutcome, MessageBackfillSource};
pub use content::{
    DownloadedMessageContent, MessageContentReadFailure, MessageContentReadFailureKind,
    MessageContentReadGateway, MessageContentReadRequest, OpenMessageContentDependencies,
    OpenMessageContentFailure, OpenMessageContentFailureKind, OpenMessageContentRequest,
    OpenMessageContentService, OpenedMessageBody, OpenedMessageContent,
};
pub use crypto::{
    DecryptMessageContentRequest, EncryptMessageContentRequest, EncryptedMessageContent,
    MessageBodyProtectionService, MessageContentCipher, MessageContentCryptographyFailure,
    MessageContentCryptographyFailureKind, ProtectMessageBodyFailure,
    ProtectMessageBodyFailureKind, ProtectMessageBodyRequest,
};
pub use incoming::{
    MessageAuthenticationDecision, MessageAuthenticationFailure, MessageAuthenticationFailureKind,
    MessageEventAuthenticator, MessageSyncDependencies, MessageSyncFailure, MessageSyncFailureKind,
    MessageSyncOutcome, MessageSyncService, is_undecryptable,
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
    InboxAcknowledgement, IsolatedSession, MessageBackfillBatch, MessageContentSourceQuery,
    MessageLookupId, MessagePreviewPage, MessagePreviewQuery, MessagePreviewQueryError,
    MessageProjectionBatch, MessageProjectionMutation, MessageProjectionStoreFailure,
    MessageProjectionStoreFailureKind, MessageRecoveryBatch, MessageRoomContext, MessageSyncIssue,
    MessageSyncIssueReason, MessageTimelineGap, MessageTimelineProjectionStore,
    MessageTimelineQueryFailure, MessageTimelineQueryFailureKind, MessageTimelineQueryRepository,
    MessagesAround, PendingTimelineGap, ProjectedActorInstanceVerification, ProjectedMessageActor,
    ProjectedMessagePreview, ProjectedMessageRevision, ReservedIsolatedEvent, TimelineLoss,
    TimelineLossReason, UndecryptableSession,
};
pub use recovery::{MessageRecoveryOutcome, MessageRecoverySource};
