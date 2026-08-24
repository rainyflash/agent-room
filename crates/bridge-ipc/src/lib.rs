mod authentication;
mod codec;
mod wire;

pub use authentication::{
    IpcAuthenticationFailure, IpcChallenge, IpcChallengeProof, IpcSharedSecret,
    create_challenge_proof, verify_challenge_proof,
};
pub use codec::{IpcFrameCodec, IpcProtocolFailure, IpcProtocolFailureKind};
pub use wire::{
    IpcCaller, IpcErrorCategory, IpcFrame, IpcMethod, IpcScopeName, IpcVersion,
    client_offer_from_frame, server_agreement_frame,
};
