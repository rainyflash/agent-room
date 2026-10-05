mod authentication;
mod client;
mod codec;
mod host_sessions;
mod matrix_recovery;
mod matrix_security;
pub use matrix_recovery::{
    IpcAgentRecoverySession, IpcMatrixRecoveryRequest, IpcMatrixRecoveryResult,
    IpcMatrixRecoveryState, MatrixRecoverySecret,
};
pub use matrix_security::{
    IpcMatrixIdentityState, IpcMatrixSecurityDevice, IpcMatrixSecurityRequest,
    IpcMatrixSecurityResult, IpcMatrixVerificationStage, IpcMatrixVerificationStep,
};
pub mod limits;
mod paths;
pub mod previews;
pub mod typing;
pub mod wake;
pub use paths::attachment_directory;
mod tools;
pub use agent_room_application::reception::{
    ReceptionCommand, ReceptionPending, ReceptionProgress, ReceptionRecord, ReceptionRequest,
    ReceptionStatus,
};
pub use host_sessions::{
    IpcCloseHostSessionRequest, IpcHostRoomTarget, IpcHostSessionDiagnostics, IpcHostSessionState,
    IpcHostSessionSummary, IpcInvitationOffer, IpcOpenHostSessionRequest, IpcPendingInvitation,
    IpcReceptionHost, IpcReceptionOffer, IpcRedeemJoinCodeRequest, IpcRegisterReceptionRequest,
    IpcResolveJoinCodeRequest, IpcWithdrawInvitationRequest,
};
mod wire;

pub use agent_room_bridge_core::room_directory::{NamedRoom, resolve_room_by_name};
pub use authentication::{
    IpcAuthenticationFailure, IpcChallenge, IpcChallengeProof, IpcSharedSecret,
    create_challenge_proof, verify_challenge_proof,
};
pub use client::{IpcClientCredentials, IpcClientFailure, IpcClientFailureKind, IpcClientSession};
pub use codec::{IpcFrameCodec, IpcProtocolFailure, IpcProtocolFailureKind};
pub use tools::{
    IpcAckInboxRequest, IpcActorSummary, IpcAgentArchiveReason, IpcAgentConnection,
    IpcAgentLifecycle, IpcAgentReception, IpcAgentSummary, IpcApproveHandoffRequest,
    IpcBootstrapDefaultAgentRequest, IpcBridgeState, IpcConsumedHandoff,
    IpcConsumedTargetedHandoff, IpcContentReference, IpcConversationMessage, IpcDeclinedHandoff,
    IpcDeclinedTargetedHandoff, IpcDefaultAgentBootstrap, IpcGetMessagesRequest,
    IpcGetPresenceRequest, IpcHandoffPermission, IpcHandoffPurpose, IpcHandoffRequest,
    IpcHandoffStatus, IpcHandoffSubmission, IpcHumanHandoffSource, IpcListHandoffsRequest,
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity,
    IpcMessagesAroundRequest, IpcMethod, IpcMethodValidationFailure, IpcOpenContentRequest,
    IpcOpenedAttachment, IpcOpenedContent, IpcOwnerSummary, IpcPendingTargetedHandoff,
    IpcPresenceSummary, IpcPublishStatusRequest, IpcPublishedStatus, IpcReplyExcerpt, IpcResponse,
    IpcRoomHistoryRequest, IpcRoomKind, IpcRoomMembership, IpcRoomSummary, IpcSelfSummary,
    IpcSendMessageRequest, IpcSentMessage, IpcSubmissionState, IpcWorkStatus,
};
pub use wire::{
    IpcCaller, IpcErrorCategory, IpcFrame, IpcScopeName, IpcVersion, client_offer_from_frame,
    server_agreement_frame,
};
