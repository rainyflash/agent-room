//! 人的设备自动签名（ADR 0011）：控制面替账户保管的密钥存储钥匙用 AES-256-GCM 封存。封存密钥和
//! 网络 Agent 共用部署里的同一把，按用途派生出另一把子密钥；附加数据绑定主体与密钥版本，密文挪给
//! 别的账户或别的用途都打不开。

use aes_gcm::{
    Aes256Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use agent_room_application::ports::{
    AccountEncryptionKeySealer, SealedSecret, SecretSealingFailure,
};
use agent_room_domain::ids::PrincipalId;
use zeroize::Zeroizing;

use crate::network_agents::NetworkAgentSealKey;

const NONCE_BYTES: usize = 12;
const AUTHENTICATION_TAG_BYTES: usize = 16;
/// 现在只有一把密钥；换密钥时新封存的钥匙带新版本号，旧的仍按旧版本打开。
const KEY_VERSION: u16 = 1;
const SEAL_KEY_DOMAIN: &[u8] = b"agent-room:account:encryption-key-seal:v1\0";
const ASSOCIATED_DATA_DOMAIN: &[u8] = b"agent-room:account:encryption-key:v1\0";

/// 没配封存密钥时一律报不可用：那时设备上的自动签名退回到只给第一台设备建立身份。
pub struct AesGcmAccountEncryptionKeySealer {
    key: Option<Zeroizing<[u8; 32]>>,
}

impl AesGcmAccountEncryptionKeySealer {
    pub fn new(key: Option<&NetworkAgentSealKey>) -> Self {
        Self {
            key: key.and_then(|key| key.derive(SEAL_KEY_DOMAIN)),
        }
    }

    fn cipher(&self) -> Result<Aes256Gcm, SecretSealingFailure> {
        let key = self.key.as_ref().ok_or(SecretSealingFailure::Unavailable)?;
        Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| SecretSealingFailure::Unavailable)
    }
}

impl AccountEncryptionKeySealer for AesGcmAccountEncryptionKeySealer {
    fn seal(
        &self,
        principal_id: PrincipalId,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure> {
        let cipher = self.cipher()?;
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| SecretSealingFailure::Unavailable)?;
        let aad = associated_data(principal_id, KEY_VERSION);
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecretSealingFailure::Unavailable)?;
        let mut bytes = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&ciphertext);
        Ok(SealedSecret {
            key_version: KEY_VERSION,
            bytes,
        })
    }

    fn open(
        &self,
        principal_id: PrincipalId,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure> {
        if sealed.key_version != KEY_VERSION
            || sealed.bytes.len() <= NONCE_BYTES + AUTHENTICATION_TAG_BYTES
        {
            return Err(SecretSealingFailure::Corrupt);
        }
        let cipher = self.cipher()?;
        let (nonce, ciphertext) = sealed.bytes.split_at(NONCE_BYTES);
        let aad = associated_data(principal_id, sealed.key_version);
        cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecretSealingFailure::Corrupt)
    }
}

fn associated_data(principal_id: PrincipalId, version: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(ASSOCIATED_DATA_DOMAIN.len() + 16 + 2);
    data.extend_from_slice(ASSOCIATED_DATA_DOMAIN);
    data.extend_from_slice(principal_id.as_uuid().as_bytes());
    data.extend_from_slice(&version.to_be_bytes());
    data
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        AccountEncryptionKeySealer as _, NetworkAgentSecretKind, NetworkAgentSecretSealer as _,
        SecretSealingFailure,
    };
    use agent_room_domain::ids::{NetworkAgentId, PrincipalId};
    use uuid::Uuid;

    use super::AesGcmAccountEncryptionKeySealer;
    use crate::network_agents::{AesGcmNetworkAgentSealer, NetworkAgentSealKey};

    fn principal() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    #[test]
    fn 封存的钥匙只有同一个账户打得开() {
        let key = NetworkAgentSealKey::from_bytes([3; 32]);
        let vault = AesGcmAccountEncryptionKeySealer::new(Some(&key));
        let owner = principal();

        let sealed = vault.seal(owner, &[7; 32]).unwrap();
        assert_ne!(sealed.bytes[12..44], [7; 32], "存的不是明文");
        assert_eq!(vault.open(owner, &sealed).unwrap(), vec![7; 32]);
        assert_eq!(
            vault.open(principal(), &sealed),
            Err(SecretSealingFailure::Corrupt),
            "挪给别的账户打不开"
        );
        let mut tampered = sealed.clone();
        if let Some(last) = tampered.bytes.last_mut() {
            *last ^= 1;
        }
        assert_eq!(
            vault.open(owner, &tampered),
            Err(SecretSealingFailure::Corrupt)
        );
    }

    #[test]
    fn 和网络_agent_的秘密用不同的子密钥_互相打不开() {
        let key = NetworkAgentSealKey::from_bytes([3; 32]);
        let account = AesGcmAccountEncryptionKeySealer::new(Some(&key));
        let agents = AesGcmNetworkAgentSealer::new(Some(&key));
        let id = Uuid::now_v7();

        let sealed = account.seal(PrincipalId::from_uuid(id), &[7; 32]).unwrap();
        assert_eq!(
            agents.open(
                NetworkAgentId::from_uuid(id),
                NetworkAgentSecretKind::MatrixRecoveryKey,
                &sealed
            ),
            Err(SecretSealingFailure::Corrupt)
        );
    }

    #[test]
    fn 没配封存密钥时报不可用() {
        let sealer = AesGcmAccountEncryptionKeySealer::new(None);
        assert_eq!(
            sealer.seal(principal(), &[7; 32]),
            Err(SecretSealingFailure::Unavailable)
        );
    }
}
