use agent_room_bridge_core::ipc::{
    IpcCallerKind, IpcHandshakeAgreement, IpcHandshakeOffer, IpcInstallationId, IpcScope,
};
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use uuid::Uuid;

const TRANSCRIPT_DOMAIN: &[u8] = b"agent-room-ipc-handshake-v1\0";

#[derive(Clone)]
pub struct IpcSharedSecret([u8; 32]);

impl IpcSharedSecret {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcChallenge([u8; 32]);

impl IpcChallenge {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcChallengeProof([u8; 32]);

impl IpcChallengeProof {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcAuthenticationFailure;

/// 为已经协商的握手内容生成 HMAC-SHA256 安装身份挑战证明。
///
/// # Errors
///
/// 底层密码学实现拒绝密钥初始化时返回错误。
pub fn create_challenge_proof(
    secret: &IpcSharedSecret,
    challenge_id: Uuid,
    challenge: IpcChallenge,
    installation_id: &IpcInstallationId,
    offer: &IpcHandshakeOffer,
    agreement: &IpcHandshakeAgreement,
) -> Result<IpcChallengeProof, IpcAuthenticationFailure> {
    let transcript =
        handshake_transcript(challenge_id, challenge, installation_id, offer, agreement);
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| IpcAuthenticationFailure)?;
    mac.update(&transcript);
    Ok(IpcChallengeProof::new(mac.finalize().into_bytes().into()))
}

/// 以常量时间校验安装身份挑战证明。
pub fn verify_challenge_proof(
    secret: &IpcSharedSecret,
    challenge_id: Uuid,
    challenge: IpcChallenge,
    installation_id: &IpcInstallationId,
    offer: &IpcHandshakeOffer,
    agreement: &IpcHandshakeAgreement,
    proof: IpcChallengeProof,
) -> bool {
    let transcript =
        handshake_transcript(challenge_id, challenge, installation_id, offer, agreement);
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(&transcript);
    mac.verify_slice(proof.as_bytes()).is_ok()
}

fn handshake_transcript(
    challenge_id: Uuid,
    challenge: IpcChallenge,
    installation_id: &IpcInstallationId,
    offer: &IpcHandshakeOffer,
    agreement: &IpcHandshakeAgreement,
) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(256);
    transcript.extend_from_slice(TRANSCRIPT_DOMAIN);
    transcript.extend_from_slice(challenge_id.as_bytes());
    transcript.extend_from_slice(challenge.as_bytes());
    append_text(&mut transcript, installation_id.as_str());
    transcript.push(caller_tag(offer.caller()));
    for version in offer.supported_versions() {
        transcript.extend_from_slice(&version.major().to_be_bytes());
        transcript.extend_from_slice(&version.minor().to_be_bytes());
    }
    transcript.push(0xff);
    transcript.extend_from_slice(&agreement.selected_version().major().to_be_bytes());
    transcript.extend_from_slice(&agreement.selected_version().minor().to_be_bytes());
    for scope in agreement.granted_scopes() {
        transcript.push(scope_tag(*scope));
    }
    transcript
}

fn append_text(buffer: &mut Vec<u8>, value: &str) {
    let length = u16::try_from(value.len()).expect("已校验安装标识长度必须适合 u16");
    buffer.extend_from_slice(&length.to_be_bytes());
    buffer.extend_from_slice(value.as_bytes());
}

const fn caller_tag(caller: IpcCallerKind) -> u8 {
    match caller {
        IpcCallerKind::McpServer => 1,
        IpcCallerKind::DesktopShell => 2,
        IpcCallerKind::DiagnosticCli => 3,
    }
}

const fn scope_tag(scope: IpcScope) -> u8 {
    match scope {
        IpcScope::BridgeStatusRead => 1,
        IpcScope::SelfRead => 2,
        IpcScope::PreviewsRead => 3,
        IpcScope::PresenceRead => 4,
        IpcScope::ContentRead => 5,
        IpcScope::StatusPublish => 6,
        IpcScope::MessageSend => 7,
        IpcScope::HandoffConsume => 8,
        IpcScope::HandoffDecline => 9,
        IpcScope::HandoffApprove => 10,
        IpcScope::AgentBootstrap => 11,
    }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::{
        FoundationIpcScopePolicy, IpcCallerKind, IpcHandshakeNegotiator, IpcHandshakeOffer,
        IpcInstallationId, IpcProtocolVersion, IpcScope,
    };
    use uuid::Uuid;

    use super::{IpcChallenge, IpcSharedSecret, create_challenge_proof, verify_challenge_proof};

    #[test]
    fn 证明绑定安装标识调用方版本和作用域() {
        let secret = IpcSharedSecret::new([7; 32]);
        let challenge_id = Uuid::from_u128(1);
        let challenge = IpcChallenge::new([9; 32]);
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let original_offer = offer(IpcCallerKind::McpServer);
        let original_agreement = agreement(&original_offer);
        let proof = create_challenge_proof(
            &secret,
            challenge_id,
            challenge,
            &installation_id,
            &original_offer,
            &original_agreement,
        )
        .expect("测试密钥可初始化 HMAC");

        assert!(verify_challenge_proof(
            &secret,
            challenge_id,
            challenge,
            &installation_id,
            &original_offer,
            &original_agreement,
            proof,
        ));

        let altered_offer = offer(IpcCallerKind::DesktopShell);
        let altered_agreement = agreement(&altered_offer);
        assert!(!verify_challenge_proof(
            &secret,
            challenge_id,
            challenge,
            &installation_id,
            &altered_offer,
            &altered_agreement,
            proof,
        ));
    }

    fn offer(caller: IpcCallerKind) -> IpcHandshakeOffer {
        IpcHandshakeOffer::new(
            caller,
            [IpcProtocolVersion::V1_0],
            [IpcScope::BridgeStatusRead],
        )
        .expect("测试握手有效")
    }

    fn agreement(offer: &IpcHandshakeOffer) -> agent_room_bridge_core::ipc::IpcHandshakeAgreement {
        IpcHandshakeNegotiator::new([IpcProtocolVersion::V1_0], FoundationIpcScopePolicy)
            .expect("测试协商器有效")
            .negotiate(offer)
            .expect("测试握手可协商")
    }
}
