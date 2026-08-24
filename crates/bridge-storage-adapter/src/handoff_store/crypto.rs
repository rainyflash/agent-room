use std::{fmt, sync::Arc};

use agent_room_bridge_core::handoffs::{
    HandoffStoreFailure, HandoffStoreFailureKind, OneShotHandoffPackage,
};
use agent_room_domain::handoff::ContextHandoff;
use chacha20poly1305::{
    Key, KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

pub const HANDOFF_STORAGE_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const KEY_VERSION: i64 = 1;
const ASSOCIATED_DATA_DOMAIN: &[u8] = b"agent-room:handoff-package:v1\0";

#[derive(Clone)]
pub struct HandoffStorageKey {
    bytes: Zeroizing<[u8; HANDOFF_STORAGE_KEY_BYTES]>,
}

impl HandoffStorageKey {
    pub fn from_bytes(bytes: [u8; HANDOFF_STORAGE_KEY_BYTES]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// 使用操作系统密码学随机源生成新的本地 Handoff 存储密钥。
    ///
    /// # Errors
    ///
    /// 操作系统随机源不可用时返回错误。
    pub fn generate() -> Result<Self, HandoffStorageKeyGenerationFailure> {
        let mut bytes = Zeroizing::new([0_u8; HANDOFF_STORAGE_KEY_BYTES]);
        getrandom::fill(&mut *bytes).map_err(HandoffStorageKeyGenerationFailure)?;
        Ok(Self { bytes })
    }
}

impl fmt::Debug for HandoffStorageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandoffStorageKey")
            .field("bytes", &"[已隐藏]")
            .finish()
    }
}

#[derive(Debug, Error)]
#[error("无法从操作系统随机源生成 Handoff 存储密钥：{0}")]
pub struct HandoffStorageKeyGenerationFailure(getrandom::Error);

pub(super) struct EncryptedPackage {
    pub key_version: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct HandoffPackageCipher {
    cipher: Arc<XChaCha20Poly1305>,
}

impl HandoffPackageCipher {
    pub fn new(key: &HandoffStorageKey) -> Self {
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.bytes[..]));
        Self {
            cipher: Arc::new(cipher),
        }
    }

    pub fn validate_plaintext(
        handoff: &ContextHandoff,
        package: &OneShotHandoffPackage,
    ) -> Result<(), HandoffStoreFailure> {
        if package.handoff_id() != handoff.fields().id {
            return Err(corrupt_failure());
        }
        let expected_length = usize::try_from(handoff.fields().content.byte_length().value())
            .map_err(|_| corrupt_failure())?;
        if package.body().len() != expected_length {
            return Err(corrupt_failure());
        }
        let actual_digest: [u8; 32] = Sha256::digest(package.body().as_ref()).into();
        if actual_digest != *handoff.fields().content.digest().as_bytes() {
            return Err(corrupt_failure());
        }
        Ok(())
    }

    pub fn encrypt(
        &self,
        handoff: &ContextHandoff,
        plaintext: &[u8],
    ) -> Result<EncryptedPackage, HandoffStoreFailure> {
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| unavailable_failure())?;
        let associated_data = associated_data(handoff);
        let ciphertext = self
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &associated_data,
                },
            )
            .map_err(|_| unavailable_failure())?;
        Ok(EncryptedPackage {
            key_version: KEY_VERSION,
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    pub fn decrypt(
        &self,
        handoff: &ContextHandoff,
        key_version: i64,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<Arc<[u8]>, HandoffStoreFailure> {
        if key_version != KEY_VERSION || nonce.len() != NONCE_BYTES {
            return Err(corrupt_failure());
        }
        let associated_data = associated_data(handoff);
        let plaintext = self
            .cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &associated_data,
                },
            )
            .map_err(|_| corrupt_failure())?;
        let package = OneShotHandoffPackage::new(handoff.fields().id, Arc::from(plaintext));
        Self::validate_plaintext(handoff, &package)?;
        Ok(package.body().clone())
    }
}

impl fmt::Debug for HandoffPackageCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandoffPackageCipher")
            .field("key", &"[已隐藏]")
            .finish()
    }
}

fn associated_data(handoff: &ContextHandoff) -> Vec<u8> {
    let fields = handoff.fields();
    let media_type = fields.content.media_type().as_str().as_bytes();
    let mut data =
        Vec::with_capacity(ASSOCIATED_DATA_DOMAIN.len() + 16 * 3 + 32 + 8 + 8 + media_type.len());
    data.extend_from_slice(ASSOCIATED_DATA_DOMAIN);
    data.extend_from_slice(fields.id.as_uuid().as_bytes());
    data.extend_from_slice(fields.target_instance_id.as_uuid().as_bytes());
    data.extend_from_slice(fields.content.content_id().as_uuid().as_bytes());
    data.extend_from_slice(fields.content.digest().as_bytes());
    data.extend_from_slice(&fields.content.byte_length().value().to_be_bytes());
    data.extend_from_slice(
        &u64::try_from(media_type.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    data.extend_from_slice(media_type);
    data
}

const fn corrupt_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Corrupt)
}

const fn unavailable_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::HandoffStorageKey;

    #[test]
    fn 调试输出不会泄露存储密钥() {
        let key = HandoffStorageKey::from_bytes([7; 32]);
        let rendered = format!("{key:?}");

        assert!(rendered.contains("已隐藏"));
        assert!(!rendered.contains("7, 7"));
    }
}
