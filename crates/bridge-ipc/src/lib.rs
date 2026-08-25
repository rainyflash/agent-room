mod authentication;
mod client;
mod codec;
pub mod limits;
mod tools;
mod wire;

pub use authentication::{
    IpcAuthenticationFailure, IpcChallenge, IpcChallengeProof, IpcSharedSecret,
    create_challenge_proof, verify_challenge_proof,
};
pub use client::{IpcClientCredentials, IpcClientFailure, IpcClientFailureKind, IpcClientSession};
pub use codec::{IpcFrameCodec, IpcProtocolFailure, IpcProtocolFailureKind};
pub use tools::{
    IpcActorSummary, IpcAgentSummary, IpcApproveHandoffRequest, IpcBridgeState, IpcConsumedHandoff,
    IpcContentReference, IpcDeclinedHandoff, IpcGetPresenceRequest, IpcHandoffPermission,
    IpcHandoffPurpose, IpcHandoffRequest, IpcHandoffStatus, IpcHandoffSubmission,
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity,
    IpcMethod, IpcMethodValidationFailure, IpcOpenContentRequest, IpcOpenedContent,
    IpcPresenceSummary, IpcPublishStatusRequest, IpcPublishedStatus, IpcResponse, IpcSelfSummary,
    IpcSendMessageRequest, IpcSentMessage, IpcSubmissionState, IpcWorkStatus,
};
pub use wire::{
    IpcCaller, IpcErrorCategory, IpcFrame, IpcScopeName, IpcVersion, client_offer_from_frame,
    server_agreement_frame,
};
