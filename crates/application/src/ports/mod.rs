mod agent_cards;
mod agents;
mod audit;
mod content;
mod devices;
mod identity;
mod matrix;
mod notifications;
mod outbox;
mod projections;
mod rooms;
mod runtime;

use std::{future::Future, pin::Pin};

pub use agent_cards::{
    AgentCardFetchFailure, AgentCardFetchFailureKind, AgentCardFetchResult,
    AgentCardSnapshotRepository, AgentCardSource, FetchedAgentCard,
};
pub use agents::{
    AgentCreationClaim, AgentCreationReservation, AgentCreationWorkflow, AgentInstanceRegistration,
    AgentInstanceRegistrationTransaction, AgentMembershipChange, AgentMembershipRepository,
    AgentMembershipTransaction, AgentRegistration, AgentRegistrationTransaction, AgentRepository,
    RegisteredAgent, StoredAgentInstanceRegistration,
};
pub use audit::{AuditRecord, AuditSink};
pub use content::ContentStore;
pub use devices::{
    DeviceProofNonceStore, DeviceProofValueError, DeviceProofVerifier, DeviceRefreshContext,
    DeviceRefreshOutcome, DeviceRegistrationTransaction, DeviceRepository, DeviceRevocationOutcome,
    DeviceRevocationTransaction, DeviceSecurityEvent, DeviceSessionRegistration,
    DeviceSessionStore, DeviceSignature, DeviceTokenReplacement, StoredDeviceSession,
};
pub use identity::{
    IdentityValueError, LoginAttempt, LoginAttemptStore, LoginCompletionTransaction,
    OidcAuthorizationOptions, OidcAuthorizationRequest, OidcCodeExchange,
    OidcDeviceAssertionVerifier, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
    OidcDeviceGrantGateway, OidcDevicePromptFailure, OidcFailure, OidcFailureKind, OidcGateway,
    OidcResult, PrincipalAccount, PrincipalRegistration, PrincipalRepository,
    PrincipalSuspensionTransaction, ProfileImportConsent, SafeReturnPath, SecretDigest,
    SecretFactory, SecretGenerationFailure, SecretValue, StoredWebSession, VerifiedOidcIdentity,
    WebSessionRegistration, WebSessionStore,
};
pub use matrix::{
    MatrixAcceptedEvent, MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner,
    MatrixAgentLocalpart, MatrixAgentUserRegistration, MatrixBackfillPage, MatrixBackfillRequest,
    MatrixBackfillToken, MatrixClientFactory, MatrixConnection, MatrixCreateRoom, MatrixDeviceId,
    MatrixEvent, MatrixEventId, MatrixEventType, MatrixFailure, MatrixFailureKind, MatrixGateway,
    MatrixLogin, MatrixOperation, MatrixReceipt, MatrixReceiptKind, MatrixRecoveryAction,
    MatrixResult, MatrixRetryPolicy, MatrixRoomId, MatrixRoomPreset, MatrixRoomSync,
    MatrixRoomSyncKind, MatrixRoomVisibility, MatrixSession, MatrixSessionMetadata,
    MatrixStateEvent, MatrixStateKey, MatrixSyncBatch, MatrixSyncRequest, MatrixSyncToken,
    MatrixTimelineEvent, MatrixTransactionId, MatrixUserId, MatrixValueError,
};
pub use notifications::NotificationSink;
pub use outbox::{
    ClaimedOutboxEvent, OutboxBacklog, OutboxClaim, OutboxFailure, OutboxFailureOutcome,
    OutboxMessage, OutboxPublisher, OutboxRepository, PublishFailure, PublishFailureKind,
};
pub use projections::{
    ActivityScoreMillis, MatrixMembership, MatrixProjectionBatch, MatrixProjectionEvent,
    MatrixProjectionEventKind, MatrixProjectionRebuild, MatrixProjectionStore,
    MembershipProjectionLookup, MembershipReadPlan, ProjectionApplyOutcome, ProjectionCursor,
    ProjectionFreshnessPolicy, ProjectionHealth, ProjectionHealthReport, ROOM_PROJECTION_CONSUMER,
};
pub use rooms::{
    PublicLobbyDirectoryEntry, RoomAllocationEvidence, RoomAllocationMode, RoomAllocationStore,
    RoomDirectory, RoomDirectoryQuery, RoomMembershipGateway, RoomReservationClaim,
    RoomReservationOutcome,
};
pub use runtime::{Clock, IdentifierFactory};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
