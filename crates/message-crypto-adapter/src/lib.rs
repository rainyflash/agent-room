use std::{fmt, sync::Arc};

use aes_gcm::{
    Aes256Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use agent_room_bridge_core::messages::{
    DecryptMessageContentRequest, EncryptMessageContentRequest, EncryptedMessageContent,
    MessageContentCipher, MessageContentCryptographyFailure, MessageContentCryptographyFailureKind,
};
use agent_room_domain::messages::{
    CLIENT_CONTENT_KEY_BYTES, CLIENT_CONTENT_NONCE_BYTES, ClientContentEncryption,
    ClientContentEncryptionAlgorithm,
};
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};
use uuid::Version;
use zeroize::Zeroizing;

pub const MESSAGE_CONTENT_ROOT_KEY_BYTES: usize = 32;
const AUTHENTICATION_TAG_BYTES: usize = 16;
const KEY_DERIVATION_DOMAIN: &[u8] = b"agent-room:message-content:key:v1\0";
const NONCE_DERIVATION_DOMAIN: &[u8] = b"agent-room:message-content:nonce:v1\0";
const ASSOCIATED_DATA_DOMAIN: &[u8] = b"agent-room:message-content:aad:v1\0";

#[derive(Clone)]
pub struct MessageContentRootKey {
    bytes: Arc<Zeroizing<[u8; MESSAGE_CONTENT_ROOT_KEY_BYTES]>>,
}

impl MessageContentRootKey {
    pub fn from_bytes(bytes: [u8; MESSAGE_CONTENT_ROOT_KEY_BYTES]) -> Self {
        Self {
            bytes: Arc::new(Zeroizing::new(bytes)),
        }
    }

    /// 使用操作系统密码学随机源生成设备正文根密钥。
    ///
    /// # Errors
    ///
    /// 操作系统随机源不可用时返回错误。
    pub fn generate() -> Result<Self, MessageContentCryptographyFailure> {
        let mut bytes = [0_u8; MESSAGE_CONTENT_ROOT_KEY_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| unavailable())?;
        Ok(Self::from_bytes(bytes))
    }
}

impl fmt::Debug for MessageContentRootKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageContentRootKey")
            .field("bytes", &"[已隐藏]")
            .finish()
    }
}

#[derive(Clone)]
pub struct AesGcmMessageContentCipher {
    root_key: MessageContentRootKey,
}

impl AesGcmMessageContentCipher {
    pub const fn new(root_key: MessageContentRootKey) -> Self {
        Self { root_key }
    }
}

impl MessageContentCipher for AesGcmMessageContentCipher {
    fn encrypt(
        &self,
        request: &EncryptMessageContentRequest<'_>,
    ) -> Result<EncryptedMessageContent, MessageContentCryptographyFailure> {
        validate_encrypt_request(request)?;
        let plaintext_size_bytes = u64::try_from(request.plaintext.len()).map_err(|_| invalid())?;
        let derivation_context = derivation_context(request, plaintext_size_bytes)?;
        let key = derive::<CLIENT_CONTENT_KEY_BYTES>(
            &self.root_key.bytes.as_ref()[..],
            KEY_DERIVATION_DOMAIN,
            &derivation_context,
        )?;
        let nonce = derive::<CLIENT_CONTENT_NONCE_BYTES>(
            &self.root_key.bytes.as_ref()[..],
            NONCE_DERIVATION_DOMAIN,
            &derivation_context,
        )?;
        let associated_data = associated_data(
            request.context_id.as_uuid().as_bytes(),
            request.room_id.as_str(),
            request.media_type.as_str(),
            plaintext_size_bytes,
        )?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| unavailable())?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: request.plaintext,
                    aad: &associated_data,
                },
            )
            .map_err(|_| unavailable())?;
        let encryption = ClientContentEncryption::new(
            ClientContentEncryptionAlgorithm::Aes256GcmV1,
            request.context_id,
            key,
            nonce,
            plaintext_size_bytes,
        )
        .map_err(|_| invalid())?;
        Ok(EncryptedMessageContent {
            ciphertext: Arc::from(ciphertext),
            encryption,
        })
    }

    fn decrypt(
        &self,
        request: &DecryptMessageContentRequest<'_>,
    ) -> Result<Arc<[u8]>, MessageContentCryptographyFailure> {
        let encryption = request.encryption;
        if encryption.algorithm() != ClientContentEncryptionAlgorithm::Aes256GcmV1
            || request.ciphertext.len()
                != usize::try_from(encryption.plaintext_size_bytes())
                    .ok()
                    .and_then(|length| length.checked_add(AUTHENTICATION_TAG_BYTES))
                    .ok_or_else(invalid)?
        {
            return Err(invalid());
        }
        let associated_data = associated_data(
            encryption.context_id().as_uuid().as_bytes(),
            request.room_id.as_str(),
            request.media_type.as_str(),
            encryption.plaintext_size_bytes(),
        )?;
        let cipher = Aes256Gcm::new_from_slice(encryption.key()).map_err(|_| invalid())?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(encryption.nonce()),
                Payload {
                    msg: request.ciphertext,
                    aad: &associated_data,
                },
            )
            .map_err(|_| authentication_failed())?;
        if u64::try_from(plaintext.len()).ok() != Some(encryption.plaintext_size_bytes()) {
            return Err(authentication_failed());
        }
        Ok(Arc::from(plaintext))
    }
}

fn validate_encrypt_request(
    request: &EncryptMessageContentRequest<'_>,
) -> Result<(), MessageContentCryptographyFailure> {
    if request.plaintext.is_empty()
        || request.context_id.as_uuid().get_version() != Some(Version::SortRand)
    {
        return Err(invalid());
    }
    Ok(())
}

fn derivation_context(
    request: &EncryptMessageContentRequest<'_>,
    plaintext_size_bytes: u64,
) -> Result<Vec<u8>, MessageContentCryptographyFailure> {
    let digest = Sha256::digest(request.plaintext);
    let mut context = associated_data(
        request.context_id.as_uuid().as_bytes(),
        request.room_id.as_str(),
        request.media_type.as_str(),
        plaintext_size_bytes,
    )?;
    context.extend_from_slice(&digest);
    Ok(context)
}

fn associated_data(
    context_id: &[u8; 16],
    room_id: &str,
    media_type: &str,
    plaintext_size_bytes: u64,
) -> Result<Vec<u8>, MessageContentCryptographyFailure> {
    let room_length = u64::try_from(room_id.len()).map_err(|_| invalid())?;
    let media_type_length = u64::try_from(media_type.len()).map_err(|_| invalid())?;
    let mut data = Vec::with_capacity(
        ASSOCIATED_DATA_DOMAIN.len() + 16 + 8 + room_id.len() + 8 + media_type.len() + 8,
    );
    data.extend_from_slice(ASSOCIATED_DATA_DOMAIN);
    data.extend_from_slice(context_id);
    data.extend_from_slice(&room_length.to_be_bytes());
    data.extend_from_slice(room_id.as_bytes());
    data.extend_from_slice(&media_type_length.to_be_bytes());
    data.extend_from_slice(media_type.as_bytes());
    data.extend_from_slice(&plaintext_size_bytes.to_be_bytes());
    Ok(data)
}

fn derive<const LENGTH: usize>(
    root_key: &[u8],
    domain: &[u8],
    context: &[u8],
) -> Result<[u8; LENGTH], MessageContentCryptographyFailure> {
    if LENGTH > 32 {
        return Err(invalid());
    }
    let mut mac =
        <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(root_key).map_err(|_| unavailable())?;
    mac.update(domain);
    mac.update(context);
    let output = mac.finalize().into_bytes();
    let mut derived = [0_u8; LENGTH];
    derived.copy_from_slice(&output[..LENGTH]);
    Ok(derived)
}

const fn invalid() -> MessageContentCryptographyFailure {
    MessageContentCryptographyFailure::new(MessageContentCryptographyFailureKind::InvalidRequest)
}

const fn authentication_failed() -> MessageContentCryptographyFailure {
    MessageContentCryptographyFailure::new(
        MessageContentCryptographyFailureKind::AuthenticationFailed,
    )
}

const fn unavailable() -> MessageContentCryptographyFailure {
    MessageContentCryptographyFailure::new(MessageContentCryptographyFailureKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_application::ports::MatrixRoomEncryption;
    use agent_room_application::ports::MatrixRoomId;
    use agent_room_bridge_core::messages::{
        DecryptMessageContentRequest, EncryptMessageContentRequest, EncryptedMessageContent,
        MessageBodyProtectionService, MessageContentCipher, MessageContentCryptographyFailure,
        MessageContentCryptographyFailureKind, ProtectMessageBodyFailureKind,
        ProtectMessageBodyRequest,
    };
    use agent_room_domain::{
        content::{ContentEncryptionMode, ContentMediaType},
        ids::{ContentEncryptionContextId, MessageSubmissionId},
    };
    use uuid::Uuid;

    use super::{AesGcmMessageContentCipher, MessageContentRootKey};

    #[test]
    fn 同一提交重试产生稳定密文而不同提交使用不同密钥() {
        let cipher = cipher();
        let first = encrypt(&cipher, context(1), "正文");
        let retry = encrypt(&cipher, context(1), "正文");
        let next = encrypt(&cipher, context(2), "正文");

        assert_eq!(first, retry);
        assert_ne!(first.ciphertext, next.ciphertext);
        assert_ne!(first.encryption.key(), next.encryption.key());
    }

    #[test]
    fn 解密认证房间媒体类型和密文() {
        let cipher = cipher();
        let room = room();
        let media_type = media_type();
        let encrypted = cipher
            .encrypt(&EncryptMessageContentRequest {
                context_id: context(3),
                room_id: &room,
                media_type: &media_type,
                plaintext: "机密正文".as_bytes(),
            })
            .expect("加密成功");
        let plaintext = cipher
            .decrypt(&DecryptMessageContentRequest {
                room_id: &room,
                media_type: &media_type,
                ciphertext: &encrypted.ciphertext,
                encryption: &encrypted.encryption,
            })
            .expect("正确上下文可解密");
        assert_eq!(&*plaintext, "机密正文".as_bytes());

        let wrong_room = MatrixRoomId::new("!other:example.test").expect("房间有效");
        let failure = cipher
            .decrypt(&DecryptMessageContentRequest {
                room_id: &wrong_room,
                media_type: &media_type,
                ciphertext: &encrypted.ciphertext,
                encryption: &encrypted.encryption,
            })
            .expect_err("错误房间必须认证失败");
        assert_eq!(
            failure.kind(),
            MessageContentCryptographyFailureKind::AuthenticationFailed
        );
    }

    #[test]
    fn 调试输出不泄露根密钥或消息密钥() {
        let root = MessageContentRootKey::from_bytes([7; 32]);
        let cipher = AesGcmMessageContentCipher::new(root.clone());
        let encrypted = encrypt(&cipher, context(4), "正文");

        assert!(!format!("{root:?}").contains("7, 7"));
        assert!(!format!("{:?}", encrypted.encryption).contains("7, 7"));
        assert!(format!("{:?}", encrypted.encryption).contains("已隐藏"));
    }

    #[test]
    fn 私密房间加密失败时绝不降级为明文正文() {
        let service = MessageBodyProtectionService::new(Arc::new(拒绝加密器));
        let room = room();
        let media_type = media_type();
        let failure = service
            .protect(&ProtectMessageBodyRequest {
                submission_id: submission(5),
                room_id: &room,
                room_encryption: MatrixRoomEncryption::EndToEnd,
                media_type: &media_type,
                plaintext: "绝不能明文发送".as_bytes(),
                expires_at: None,
            })
            .expect_err("加密失败必须中止发送");

        assert_eq!(failure.kind(), ProtectMessageBodyFailureKind::Cryptography);
        assert_eq!(
            failure
                .cryptography_failure()
                .expect("保留密码学失败原因")
                .kind(),
            MessageContentCryptographyFailureKind::Unavailable
        );
    }

    #[test]
    fn 公共房间保持服务端治理而私密房间生成客户端密文() {
        let cipher = Arc::new(cipher());
        let service = MessageBodyProtectionService::new(cipher.clone());
        let room = room();
        let media_type = media_type();
        let plaintext = "房间策略是唯一真相".as_bytes();
        let public = service
            .protect(&ProtectMessageBodyRequest {
                submission_id: submission(6),
                room_id: &room,
                room_encryption: MatrixRoomEncryption::Unencrypted,
                media_type: &media_type,
                plaintext,
                expires_at: None,
            })
            .expect("公共房间正文有效");
        assert_eq!(public.encryption_mode(), ContentEncryptionMode::ServerSide);
        assert_eq!(public.bytes().as_ref(), plaintext);
        assert!(public.client_encryption().is_none());

        let private = service
            .protect(&ProtectMessageBodyRequest {
                submission_id: submission(7),
                room_id: &room,
                room_encryption: MatrixRoomEncryption::EndToEnd,
                media_type: &media_type,
                plaintext,
                expires_at: None,
            })
            .expect("私密房间正文可加密");
        assert_eq!(private.encryption_mode(), ContentEncryptionMode::ClientE2ee);
        assert_ne!(private.bytes().as_ref(), plaintext);
        assert!(private.client_encryption().is_some());
    }

    fn cipher() -> AesGcmMessageContentCipher {
        AesGcmMessageContentCipher::new(MessageContentRootKey::from_bytes([9; 32]))
    }

    fn encrypt(
        cipher: &AesGcmMessageContentCipher,
        context_id: ContentEncryptionContextId,
        body: &str,
    ) -> EncryptedMessageContent {
        let room = room();
        let media_type = media_type();
        cipher
            .encrypt(&EncryptMessageContentRequest {
                context_id,
                room_id: &room,
                media_type: &media_type,
                plaintext: body.as_bytes(),
            })
            .expect("测试正文加密成功")
    }

    fn context(seed: u128) -> ContentEncryptionContextId {
        let mut bytes = *Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e48")
            .expect("测试上下文标识有效")
            .as_bytes();
        bytes[15] = u8::try_from(seed).expect("测试种子有效");
        ContentEncryptionContextId::from_uuid(Uuid::from_bytes(bytes))
    }

    fn submission(seed: u128) -> MessageSubmissionId {
        MessageSubmissionId::from_uuid(context(seed).as_uuid())
    }

    fn room() -> MatrixRoomId {
        MatrixRoomId::new("!private:example.test").expect("房间有效")
    }

    fn media_type() -> ContentMediaType {
        ContentMediaType::new("text/plain").expect("媒体类型有效")
    }

    struct 拒绝加密器;

    impl MessageContentCipher for 拒绝加密器 {
        fn encrypt(
            &self,
            _request: &EncryptMessageContentRequest<'_>,
        ) -> Result<EncryptedMessageContent, MessageContentCryptographyFailure> {
            Err(MessageContentCryptographyFailure::new(
                MessageContentCryptographyFailureKind::Unavailable,
            ))
        }

        fn decrypt(
            &self,
            _request: &DecryptMessageContentRequest<'_>,
        ) -> Result<Arc<[u8]>, MessageContentCryptographyFailure> {
            Err(MessageContentCryptographyFailure::new(
                MessageContentCryptographyFailureKind::Unavailable,
            ))
        }
    }
}
