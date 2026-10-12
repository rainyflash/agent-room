//! 私人房间的 Agent 口令封存后存着，管理者能再看到（`specs/network-agents/code-first.md`）。封存密钥
//! 和网络 Agent、账户钥匙共用部署里的同一把，按用途派生出另一把子密钥；附加数据绑定房间与密钥版本，
//! 密文挪到别的房间或别的用途都打不开。

use aes_gcm::{
    Aes256Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use agent_room_application::ports::{JoinCodeSealer, SealedSecret, SecretSealingFailure};
use agent_room_domain::ids::RoomCatalogId;
use zeroize::Zeroizing;

use crate::network_agents::NetworkAgentSealKey;

const NONCE_BYTES: usize = 12;
const AUTHENTICATION_TAG_BYTES: usize = 16;
/// 现在只有一把密钥；换密钥时新封存的口令带新版本号，旧的仍按旧版本打开。
const KEY_VERSION: u16 = 1;
const SEAL_KEY_DOMAIN: &[u8] = b"agent-room:private-room:join-code-seal:v1\0";
const ASSOCIATED_DATA_DOMAIN: &[u8] = b"agent-room:private-room:join-code:v1\0";

/// 没配封存密钥时一律报不可用：那时口令照样生成，只是管理者以后看不到。
pub struct AesGcmJoinCodeSealer {
    key: Option<Zeroizing<[u8; 32]>>,
}

impl AesGcmJoinCodeSealer {
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

impl JoinCodeSealer for AesGcmJoinCodeSealer {
    fn seal(
        &self,
        catalog_id: RoomCatalogId,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure> {
        let cipher = self.cipher()?;
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| SecretSealingFailure::Unavailable)?;
        let aad = associated_data(catalog_id, KEY_VERSION);
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
        catalog_id: RoomCatalogId,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure> {
        if sealed.key_version != KEY_VERSION
            || sealed.bytes.len() <= NONCE_BYTES + AUTHENTICATION_TAG_BYTES
        {
            return Err(SecretSealingFailure::Corrupt);
        }
        let cipher = self.cipher()?;
        let (nonce, ciphertext) = sealed.bytes.split_at(NONCE_BYTES);
        let aad = associated_data(catalog_id, sealed.key_version);
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

fn associated_data(catalog_id: RoomCatalogId, version: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(ASSOCIATED_DATA_DOMAIN.len() + 16 + 2);
    data.extend_from_slice(ASSOCIATED_DATA_DOMAIN);
    data.extend_from_slice(catalog_id.as_uuid().as_bytes());
    data.extend_from_slice(&version.to_be_bytes());
    data
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        AccountEncryptionKeySealer as _, JoinCodeSealer as _, SecretSealingFailure,
    };
    use agent_room_domain::ids::{PrincipalId, RoomCatalogId};
    use uuid::Uuid;

    use super::AesGcmJoinCodeSealer;
    use crate::{AesGcmAccountEncryptionKeySealer, network_agents::NetworkAgentSealKey};

    fn room() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    #[test]
    fn 封存的口令只有同一个房间打得开_改过的打不开() {
        let key = NetworkAgentSealKey::from_bytes([5; 32]);
        let vault = AesGcmJoinCodeSealer::new(Some(&key));
        let catalog_id = room();

        let sealed = vault.seal(catalog_id, b"ABCDEFGHJKMN").unwrap();
        assert!(
            !sealed
                .bytes
                .windows(12)
                .any(|window| window == b"ABCDEFGHJKMN"),
            "存的不是明文"
        );
        assert_eq!(
            vault.open(catalog_id, &sealed).unwrap(),
            b"ABCDEFGHJKMN".to_vec()
        );
        assert_eq!(
            vault.open(room(), &sealed),
            Err(SecretSealingFailure::Corrupt),
            "挪到别的房间打不开"
        );
        let mut tampered = sealed.clone();
        if let Some(last) = tampered.bytes.last_mut() {
            *last ^= 1;
        }
        assert_eq!(
            vault.open(catalog_id, &tampered),
            Err(SecretSealingFailure::Corrupt)
        );
    }

    #[test]
    fn 和账户钥匙用不同的子密钥_互相打不开() {
        let key = NetworkAgentSealKey::from_bytes([5; 32]);
        let codes = AesGcmJoinCodeSealer::new(Some(&key));
        let accounts = AesGcmAccountEncryptionKeySealer::new(Some(&key));
        let id = Uuid::now_v7();

        let sealed = codes
            .seal(RoomCatalogId::from_uuid(id), b"ABCDEFGHJKMN")
            .unwrap();
        assert_eq!(
            accounts.open(PrincipalId::from_uuid(id), &sealed),
            Err(SecretSealingFailure::Corrupt)
        );
    }

    #[test]
    fn 没配封存密钥时报不可用() {
        let sealer = AesGcmJoinCodeSealer::new(None);
        assert_eq!(
            sealer.seal(room(), b"ABCDEFGHJKMN"),
            Err(SecretSealingFailure::Unavailable)
        );
    }
}
