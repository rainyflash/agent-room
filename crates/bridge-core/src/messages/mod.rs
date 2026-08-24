mod model;
mod outgoing;
mod ports;
mod wire;

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
