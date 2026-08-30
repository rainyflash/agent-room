mod accounts;
mod agent_cards;
mod agents;
mod audit;
mod automation;
mod content;
mod devices;
mod handoffs;
mod identity;
mod matrix;
mod moderation;
mod notifications;
mod outbox;
mod projections;
mod rooms;
mod runtime;

use std::{future::Future, pin::Pin};

pub use accounts::{
    AccountDeletionClaim, AccountDeletionReceiptIssuer, AccountDeletionRepository,
    AccountDeletionRequest, AccountDeletionRequestOutcome, AccountDeletionStage,
    AccountDeletionStatus, AccountExportSnapshot,
};
pub use agent_cards::{
    AgentCardFetchFailure, AgentCardFetchFailureKind, AgentCardFetchResult,
    AgentCardSnapshotRepository, AgentCardSource, FetchedAgentCard,
};
pub use agents::{
    AgentCreationClaim, AgentCreationReservation, AgentCreationWorkflow,
    AgentInstanceManagementRecord, AgentInstanceManagementRepository,
    AgentInstanceMatrixCleanupStore, AgentInstanceRegistration,
    AgentInstanceRegistrationTransaction, AgentInstanceRevocationOutcome,
    AgentInstanceRevocationTransaction, AgentInstanceSignatureVerifier,
    AgentInstanceVerificationRecord, AgentInstanceVerificationRepository, AgentMembershipChange,
    AgentMembershipRepository, AgentMembershipTransaction, AgentRegistration,
    AgentRegistrationTransaction, AgentRepository, RegisteredAgent,
    StoredAgentInstanceRegistration,
};
pub use audit::{AuditRecord, AuditSink};
pub use automation::{
    AutomationConsumptionOutcome, AutomationConsumptionRequest, AutomationDecisionRecord,
    AutomationGrantRecord, AutomationGrantRepository, AutomationGrantRevocationOutcome,
    AutomationScopeAuthority, AutomationScopeAuthorityRequest, AutomationSendAuthority,
    AutomationSendAuthorityRequest,
};
pub use content::{
    ContentAccessMode, ContentAccessPolicy, ContentAuthorizationDecision,
    ContentAuthorizationFailure, ContentAuthorizationFailureKind, ContentAuthorizationIntent,
    ContentAuthorizationRequest, ContentAuthorizationResult, ContentByteStream,
    ContentDownloadAttempt, ContentDownloadLimiter, ContentEventBinding,
    ContentLifecycleTransition, ContentMembershipAuthorizer, ContentPrincipalIdentityLookup,
    ContentRateLimitDecision, ContentRateLimitFailure, ContentRateLimitFailureKind,
    ContentRateLimitResult, ContentReadTicket, ContentReadTicketClaims, ContentReadTicketCodec,
    ContentRepository, ContentScanFailure, ContentScanFailureKind, ContentScanResult,
    ContentScanner, ContentStorageKeyFactory, ContentStorageKeyGenerationFailure,
    ContentStorageKeyGenerationResult, ContentStreamFailure, ContentStreamFailureKind,
    ContentStreamResult, ContentTicketFailure, ContentTicketFailureKind, ContentTicketResult,
    ContentUploadClaim, ContentUploadClaimOutcome, ContentUploadFingerprint, ObjectStoreFailure,
    ObjectStoreFailureKind, ObjectStoreResult, ObjectWriteReceipt, OpenedContentObject,
    PrivateContentObjectStore, ReclaimableContentQuery,
};
pub use devices::{
    DeviceProofNonceStore, DeviceProofValueError, DeviceProofVerifier, DeviceRefreshContext,
    DeviceRefreshOutcome, DeviceRegistrationTransaction, DeviceRepository, DeviceRevocationOutcome,
    DeviceRevocationTransaction, DeviceSecurityEvent, DeviceSessionRegistration,
    DeviceSessionStore, DeviceSignature, DeviceTokenReplacement,
    PendingAgentMatrixDeviceRevocation, StoredDeviceSession,
};
pub use handoffs::{
    ClaimTargetedHandoff, HandoffAccessRepository, HandoffAuthorizationSnapshot,
    HandoffInstanceAccessRecord, QueueTargetedHandoff, QueueTargetedHandoffOutcome,
    RecordTargetedHandoffReceipt, TargetedHandoffReceiptOutcome, TargetedHandoffRepository,
    TargetedHandoffRequestFingerprint, TargetedHandoffTargetRecord,
};
pub use identity::{
    DesktopAuthorizationCodeRegistration, DesktopClientState, DesktopLoginCompletionTransaction,
    DesktopSessionExchangeTransaction, DesktopSessionRegistration, IdentityValueError,
    LoginAttempt, LoginAttemptStore, LoginCompletionTransaction, LoginDelivery,
    OidcAuthorizationOptions, OidcAuthorizationRequest, OidcCodeExchange,
    OidcDeviceAssertionVerifier, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
    OidcDeviceGrantGateway, OidcDevicePromptFailure, OidcFailure, OidcFailureKind, OidcGateway,
    OidcInteraction, OidcResult, PkceCodeChallenge, PrincipalAccount, PrincipalRegistration,
    PrincipalRepository, PrincipalSuspensionTransaction, ProfileImportConsent, SafeReturnPath,
    SecretDigest, SecretFactory, SecretGenerationFailure, SecretValue, StoredWebSession,
    VerifiedOidcIdentity, WebSessionRegistration, WebSessionStore,
};
pub use matrix::{
    MatrixAcceptedEvent, MatrixAccountLifecycleGateway, MatrixAgentDeviceSessionRequest,
    MatrixAgentDeviceSessionRevoker, MatrixAgentDeviceSessionTarget,
    MatrixAgentIdentityProvisioner, MatrixAgentLocalpart, MatrixAgentUserRegistration,
    MatrixBackfillPage, MatrixBackfillRequest, MatrixBackfillToken, MatrixClientFactory,
    MatrixConnection, MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventId,
    MatrixEventType, MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixLogin, MatrixOperation,
    MatrixPowerLevel, MatrixReceipt, MatrixReceiptKind, MatrixRecoveryAction, MatrixResult,
    MatrixRetryPolicy, MatrixRoomAliasLocalpart, MatrixRoomAuthority, MatrixRoomAuthorityGateway,
    MatrixRoomEncryption, MatrixRoomId, MatrixRoomKind, MatrixRoomPowerProfile, MatrixRoomPreset,
    MatrixRoomStatePosition, MatrixRoomSync, MatrixRoomSyncKind, MatrixRoomVisibility,
    MatrixSession, MatrixSessionMetadata, MatrixStateEvent, MatrixStateKey, MatrixSyncBatch,
    MatrixSyncRequest, MatrixSyncToken, MatrixTimelineEncryption, MatrixTimelineEvent,
    MatrixTransactionId, MatrixUserId, MatrixValueError,
};
pub use moderation::{
    ModerationActionReservationOutcome, ModerationAuthority, ModerationEffectGateway,
    ModerationEffectTarget, ModerationIdentifierFactory, ModerationReportPolicy,
    ModerationReportSubmissionOutcome, ModerationRepository, ModerationRoomContext,
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
    AgentLobbyAccessRecord, AgentLobbyAccessRepository, AgentRoomMembershipFactory,
    DirectAgentProfile, DirectMatrixRoomCreation, DirectSessionAgentDirectory,
    DirectSessionMatrixProvisioner, DirectSessionRecord, DirectSessionStore,
    PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
    PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner, PrivateRoomPrincipalDirectory,
    PrivateRoomSnapshot, PrivateRoomStore, PublicLobbyDirectoryEntry, PublicLobbyObservationRoom,
    RoomAllocationEvidence, RoomAllocationMode, RoomAllocationStore, RoomDirectory,
    RoomDirectoryQuery, RoomMembershipGateway, RoomProvisioningClaim, RoomProvisioningClaimOutcome,
    RoomProvisioningFailureCode, RoomProvisioningGateway, RoomProvisioningJob,
    RoomProvisioningKind, RoomProvisioningStore, RoomProvisioningTarget, RoomReservationClaim,
    RoomReservationOutcome,
};
pub use runtime::{Clock, IdentifierFactory};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
