use std::{fmt, sync::Arc};

use agent_room_application::ports::MatrixRoomId;
use agent_room_domain::{
    ids::ContentId,
    messages::{CLIENT_CONTENT_KEY_BYTES, ClientContentEncryption},
};
use chacha20poly1305::{
    Key, KeyInit as _, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use zeroize::Zeroizing;

pub const MESSAGE_PROJECTION_STORAGE_KEY_BYTES: usize = 32;
pub(crate) const MESSAGE_PROJECTION_WRAPPING_NONCE_BYTES: usize = 24;
const ASSOCIATED_DATA_DOMAIN: &[u8] = b"agent-room:message-projection-key:v1\0";

#[derive(Clone)]
pub struct MessageProjectionStorageKey {
    bytes: Zeroizing<[u8; MESSAGE_PROJECTION_STORAGE_KEY_BYTES]>,
}

impl MessageProjectionStorageKey {
    pub fn from_bytes(bytes: [u8; MESSAGE_PROJECTION_STORAGE_KEY_BYTES]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }
}

impl fmt::Debug for MessageProjectionStorageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageProjectionStorageKey")
            .field("bytes", &"[已隐藏]")
            .finish()
    }
}

pub(crate) struct WrappedMessageContentKey {
    pub nonce: [u8; MESSAGE_PROJECTION_WRAPPING_NONCE_BYTES],
    pub ciphertext: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct MessageProjectionKeyCipher {
    cipher: Arc<XChaCha20Poly1305>,
}

impl MessageProjectionKeyCipher {
    pub fn new(key: &MessageProjectionStorageKey) -> Self {
        Self {
            cipher: Arc::new(XChaCha20Poly1305::new(Key::from_slice(&key.bytes[..]))),
        }
    }

    pub fn wrap(
        &self,
        room_id: &MatrixRoomId,
        content_id: ContentId,
        encryption: &ClientContentEncryption,
    ) -> Result<WrappedMessageContentKey, ()> {
        let mut nonce = [0_u8; MESSAGE_PROJECTION_WRAPPING_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| ())?;
        let aad = associated_data(room_id, content_id, encryption)?;
        let ciphertext = self
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: encryption.key(),
                    aad: &aad,
                },
            )
            .map_err(|_| ())?;
        Ok(WrappedMessageContentKey { nonce, ciphertext })
    }

    pub fn unwrap(
        &self,
        room_id: &MatrixRoomId,
        content_id: ContentId,
        encryption: &ClientContentEncryption,
        nonce: &[u8; MESSAGE_PROJECTION_WRAPPING_NONCE_BYTES],
        ciphertext: &[u8],
    ) -> Result<[u8; CLIENT_CONTENT_KEY_BYTES], ()> {
        let aad = associated_data(room_id, content_id, encryption)?;
        self.cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| ())?
            .try_into()
            .map_err(|_| ())
    }
}

fn associated_data(
    room_id: &MatrixRoomId,
    content_id: ContentId,
    encryption: &ClientContentEncryption,
) -> Result<Vec<u8>, ()> {
    let room_length = u64::try_from(room_id.as_str().len()).map_err(|_| ())?;
    let mut data = Vec::with_capacity(
        ASSOCIATED_DATA_DOMAIN.len() + room_id.as_str().len() + 8 + 16 * 2 + 12 + 8,
    );
    data.extend_from_slice(ASSOCIATED_DATA_DOMAIN);
    data.extend_from_slice(&room_length.to_be_bytes());
    data.extend_from_slice(room_id.as_str().as_bytes());
    data.extend_from_slice(content_id.as_uuid().as_bytes());
    data.extend_from_slice(encryption.context_id().as_uuid().as_bytes());
    data.extend_from_slice(encryption.nonce());
    data.extend_from_slice(&encryption.plaintext_size_bytes().to_be_bytes());
    Ok(data)
}
