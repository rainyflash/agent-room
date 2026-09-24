//! 只凭网络接入的 Agent（ADR 0010）：服务器替它生成签名密钥，保管的秘密用 AES-256-GCM 封存，
//! 来源地址只存按天加盐的摘要。封存与摘要都从部署配置里的一把封存密钥派生，用途不同的子密钥
//! 互不相同。

use std::{fmt, sync::Arc};

use aes_gcm::{
    Aes256Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use agent_room_application::ports::{
    GeneratedSigningKey, NetworkAgentKeyFactory, NetworkAgentSecretKind, NetworkAgentSecretSealer,
    SealedSecret, SecretGenerationFailure, SecretSealingFailure,
};
use agent_room_domain::ids::NetworkAgentId;
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::Ed25519DeviceSigningKey;

pub const NETWORK_AGENT_SEAL_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const AUTHENTICATION_TAG_BYTES: usize = 16;
/// 现在只有一把密钥；换密钥时新封存的秘密带新版本号，旧的仍按旧版本打开。
const KEY_VERSION: u16 = 1;
const SEAL_KEY_DOMAIN: &[u8] = b"agent-room:network-agent:seal-key:v1\0";
const SOURCE_KEY_DOMAIN: &[u8] = b"agent-room:network-agent:source-key:v1\0";
const ASSOCIATED_DATA_DOMAIN: &[u8] = b"agent-room:network-agent:secret:v1\0";

/// 替网络 Agent 生成签名密钥：种子交给封存后入库，公钥登记到设备或实例上。
pub struct Ed25519NetworkAgentKeyFactory;

impl NetworkAgentKeyFactory for Ed25519NetworkAgentKeyFactory {
    fn generate(&self) -> Result<GeneratedSigningKey, SecretGenerationFailure> {
        let key = Ed25519DeviceSigningKey::generate()
            .map_err(|_| SecretGenerationFailure::EntropyUnavailable)?;
        let encoded_seed = key
            .encoded_seed()
            .map_err(|_| SecretGenerationFailure::EntropyUnavailable)?;
        let public_key = *key
            .public_key()
            .map_err(|_| SecretGenerationFailure::EntropyUnavailable)?
            .as_bytes();
        Ok(GeneratedSigningKey {
            encoded_seed,
            public_key,
        })
    }
}

/// 部署配置里的封存密钥（32 字节）。调试输出里不出现。
#[derive(Clone)]
pub struct NetworkAgentSealKey {
    bytes: Arc<Zeroizing<[u8; NETWORK_AGENT_SEAL_KEY_BYTES]>>,
}

impl NetworkAgentSealKey {
    pub fn from_bytes(bytes: [u8; NETWORK_AGENT_SEAL_KEY_BYTES]) -> Self {
        Self {
            bytes: Arc::new(Zeroizing::new(bytes)),
        }
    }

    /// 按用途派生的子密钥：封存与来源摘要不共用同一把。
    fn derive(&self, domain: &[u8]) -> Option<Zeroizing<[u8; 32]>> {
        let mut mac =
            <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.bytes.as_slice()).ok()?;
        mac.update(domain);
        let mut key = Zeroizing::new([0_u8; 32]);
        key.copy_from_slice(&mac.finalize().into_bytes());
        Some(key)
    }
}

impl fmt::Debug for NetworkAgentSealKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkAgentSealKey")
            .field("bytes", &"[已隐藏]")
            .finish()
    }
}

/// 封存网络 Agent 的秘密。附加数据绑定 Agent、秘密种类与密钥版本：密文挪给别的 Agent 或
/// 别的用途都打不开。没配封存密钥时（总开关关着）一律报不可用。
pub struct AesGcmNetworkAgentSealer {
    key: Option<Zeroizing<[u8; 32]>>,
}

impl AesGcmNetworkAgentSealer {
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

impl NetworkAgentSecretSealer for AesGcmNetworkAgentSealer {
    fn seal(
        &self,
        agent: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure> {
        let cipher = self.cipher()?;
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| SecretSealingFailure::Unavailable)?;
        let aad = associated_data(agent, kind, KEY_VERSION);
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
        agent: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure> {
        if sealed.key_version != KEY_VERSION
            || sealed.bytes.len() <= NONCE_BYTES + AUTHENTICATION_TAG_BYTES
        {
            return Err(SecretSealingFailure::Corrupt);
        }
        let cipher = self.cipher()?;
        let (nonce, ciphertext) = sealed.bytes.split_at(NONCE_BYTES);
        let aad = associated_data(agent, kind, sealed.key_version);
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

fn associated_data(agent: NetworkAgentId, kind: NetworkAgentSecretKind, version: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(ASSOCIATED_DATA_DOMAIN.len() + 16 + 32);
    data.extend_from_slice(ASSOCIATED_DATA_DOMAIN);
    data.extend_from_slice(agent.as_uuid().as_bytes());
    data.extend_from_slice(kind.as_str().as_bytes());
    data.push(0);
    data.extend_from_slice(&version.to_be_bytes());
    data
}

/// 来源地址的摘要：按 UTC 日期加盐的 HMAC，同一地址隔天就对不上，库里也看不出原地址。
/// 没配封存密钥时（总开关关着）返回全零，反正那时不会创建网络 Agent。
pub struct NetworkSourceDigester {
    key: Option<Zeroizing<[u8; 32]>>,
}

impl NetworkSourceDigester {
    pub fn new(key: Option<&NetworkAgentSealKey>) -> Self {
        Self {
            key: key.and_then(|key| key.derive(SOURCE_KEY_DOMAIN)),
        }
    }

    #[must_use]
    pub fn digest(&self, source: &str, utc_day: &str) -> [u8; 32] {
        let Some(key) = &self.key else {
            return [0; 32];
        };
        let Ok(mut mac) = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key.as_slice()) else {
            return [0; 32];
        };
        mac.update(utc_day.as_bytes());
        mac.update(&[0]);
        mac.update(source.as_bytes());
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&mac.finalize().into_bytes());
        digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> NetworkAgentSealKey {
        NetworkAgentSealKey::from_bytes([byte; NETWORK_AGENT_SEAL_KEY_BYTES])
    }

    fn agent(value: u128) -> NetworkAgentId {
        NetworkAgentId::from_uuid(uuid::Uuid::from_u128(value))
    }

    #[test]
    fn 封存后能打开_挪给别的_agent_或种类就打不开() {
        let vault = AesGcmNetworkAgentSealer::new(Some(&key(7)));
        let sealed = vault
            .seal(
                agent(1),
                NetworkAgentSecretKind::MatrixAccessToken,
                b"syt_token",
            )
            .expect("封存");
        assert_eq!(sealed.key_version, 1);
        assert!(!sealed.bytes.windows(9).any(|window| window == b"syt_token"));
        assert_eq!(
            vault
                .open(agent(1), NetworkAgentSecretKind::MatrixAccessToken, &sealed)
                .expect("打开"),
            b"syt_token"
        );
        for (other, kind) in [
            (agent(2), NetworkAgentSecretKind::MatrixAccessToken),
            (agent(1), NetworkAgentSecretKind::InstanceSigningSeed),
        ] {
            assert_eq!(
                vault.open(other, kind, &sealed),
                Err(SecretSealingFailure::Corrupt)
            );
        }
        let mut tampered = sealed.clone();
        let last = tampered.bytes.len() - 1;
        tampered.bytes[last] ^= 1;
        assert_eq!(
            vault.open(
                agent(1),
                NetworkAgentSecretKind::MatrixAccessToken,
                &tampered
            ),
            Err(SecretSealingFailure::Corrupt)
        );
        // 换了封存密钥就打不开。
        assert_eq!(
            AesGcmNetworkAgentSealer::new(Some(&key(8))).open(
                agent(1),
                NetworkAgentSecretKind::MatrixAccessToken,
                &sealed
            ),
            Err(SecretSealingFailure::Corrupt)
        );
        // 每次封存用新的随机数。
        let again = vault
            .seal(
                agent(1),
                NetworkAgentSecretKind::MatrixAccessToken,
                b"syt_token",
            )
            .expect("封存");
        assert_ne!(again.bytes, sealed.bytes);
    }

    #[test]
    fn 没配封存密钥时一律不可用() {
        let vault = AesGcmNetworkAgentSealer::new(None);
        assert_eq!(
            vault.seal(agent(1), NetworkAgentSecretKind::DeviceSigningSeed, b"seed"),
            Err(SecretSealingFailure::Unavailable)
        );
    }

    #[test]
    fn 来源摘要按天变化_不同地址不同_看不出原地址() {
        let digester = NetworkSourceDigester::new(Some(&key(7)));
        let today = digester.digest("203.0.113.9", "2026-09-23");
        assert_eq!(today, digester.digest("203.0.113.9", "2026-09-23"));
        assert_ne!(today, digester.digest("203.0.113.9", "2026-09-24"));
        assert_ne!(today, digester.digest("203.0.113.10", "2026-09-23"));
        assert_ne!(
            today,
            NetworkSourceDigester::new(Some(&key(8))).digest("203.0.113.9", "2026-09-23")
        );
        assert_eq!(
            NetworkSourceDigester::new(None).digest("203.0.113.9", "2026-09-23"),
            [0; 32]
        );
    }

    #[test]
    fn 密钥种子能还原出同一把公钥_每次生成都不同() {
        let first = Ed25519NetworkAgentKeyFactory.generate().expect("生成");
        let restored =
            Ed25519DeviceSigningKey::from_encoded_seed(&first.encoded_seed).expect("种子可还原");
        assert_eq!(
            *restored.public_key().expect("公钥").as_bytes(),
            first.public_key
        );
        let second = Ed25519NetworkAgentKeyFactory.generate().expect("生成");
        assert_ne!(first.public_key, second.public_key);
        assert_ne!(first.encoded_seed, second.encoded_seed);
    }

    #[test]
    fn 封存密钥不进调试输出() {
        assert!(!format!("{:?}", key(7)).contains('7'));
    }
}
