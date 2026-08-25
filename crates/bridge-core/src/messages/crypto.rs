use std::sync::Arc;

use agent_room_application::ports::{MatrixRoomEncryption, MatrixRoomId};
use agent_room_domain::{
    content::{ContentEncryptionMode, ContentMediaType},
    ids::{ContentEncryptionContextId, MessageSubmissionId},
    messages::ClientContentEncryption,
    time::UtcMillis,
};

use super::{MessageBody, MessageRequestError};

#[derive(Debug)]
pub struct EncryptMessageContentRequest<'a> {
    pub context_id: ContentEncryptionContextId,
    pub room_id: &'a MatrixRoomId,
    pub media_type: &'a ContentMediaType,
    pub plaintext: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedMessageContent {
    pub ciphertext: Arc<[u8]>,
    pub encryption: ClientContentEncryption,
}

#[derive(Debug)]
pub struct DecryptMessageContentRequest<'a> {
    pub room_id: &'a MatrixRoomId,
    pub media_type: &'a ContentMediaType,
    pub ciphertext: &'a [u8],
    pub encryption: &'a ClientContentEncryption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageContentCryptographyFailureKind {
    InvalidRequest,
    AuthenticationFailed,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageContentCryptographyFailure {
    kind: MessageContentCryptographyFailureKind,
}

impl MessageContentCryptographyFailure {
    pub const fn new(kind: MessageContentCryptographyFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> MessageContentCryptographyFailureKind {
        self.kind
    }
}

/// 私密正文 AEAD 端口。实现必须认证房间、媒体类型和上下文标识，且不得回退明文。
pub trait MessageContentCipher: Send + Sync {
    /// 使用请求上下文认证并加密消息正文。
    ///
    /// # Errors
    ///
    /// 请求无效、密码学依赖不可用或加密失败时返回错误。
    fn encrypt(
        &self,
        request: &EncryptMessageContentRequest<'_>,
    ) -> Result<EncryptedMessageContent, MessageContentCryptographyFailure>;

    /// 使用事件携带的客户端密钥认证并解密消息正文。
    ///
    /// # Errors
    ///
    /// 请求无效、认证失败或密码学依赖不可用时返回错误。
    fn decrypt(
        &self,
        request: &DecryptMessageContentRequest<'_>,
    ) -> Result<Arc<[u8]>, MessageContentCryptographyFailure>;
}

pub struct ProtectMessageBodyRequest<'a> {
    pub submission_id: MessageSubmissionId,
    pub room_id: &'a MatrixRoomId,
    pub room_encryption: MatrixRoomEncryption,
    pub media_type: &'a ContentMediaType,
    pub plaintext: &'a [u8],
    pub expires_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectMessageBodyFailureKind {
    InvalidBody,
    Cryptography,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectMessageBodyFailure {
    kind: ProtectMessageBodyFailureKind,
    cryptography: Option<MessageContentCryptographyFailure>,
}

impl ProtectMessageBodyFailure {
    const fn invalid_body(_failure: MessageRequestError) -> Self {
        Self {
            kind: ProtectMessageBodyFailureKind::InvalidBody,
            cryptography: None,
        }
    }

    const fn cryptography(failure: MessageContentCryptographyFailure) -> Self {
        Self {
            kind: ProtectMessageBodyFailureKind::Cryptography,
            cryptography: Some(failure),
        }
    }

    pub const fn kind(self) -> ProtectMessageBodyFailureKind {
        self.kind
    }

    pub const fn cryptography_failure(self) -> Option<MessageContentCryptographyFailure> {
        self.cryptography
    }
}

pub struct MessageBodyProtectionService {
    cipher: Arc<dyn MessageContentCipher>,
}

impl MessageBodyProtectionService {
    pub fn new(cipher: Arc<dyn MessageContentCipher>) -> Self {
        Self { cipher }
    }

    /// 依据权威房间加密策略创建上传正文；私密路径失败时绝不回退明文。
    ///
    /// # Errors
    ///
    /// 正文无效或客户端密码学失败时返回错误。
    pub fn protect(
        &self,
        request: &ProtectMessageBodyRequest<'_>,
    ) -> Result<MessageBody, ProtectMessageBodyFailure> {
        match request.room_encryption {
            MatrixRoomEncryption::Unencrypted => MessageBody::new(
                Arc::<[u8]>::from(request.plaintext),
                request.media_type.clone(),
                ContentEncryptionMode::ServerSide,
                request.expires_at,
            )
            .map_err(ProtectMessageBodyFailure::invalid_body),
            MatrixRoomEncryption::EndToEnd => {
                let protected = self
                    .cipher
                    .encrypt(&EncryptMessageContentRequest {
                        context_id: ContentEncryptionContextId::from_uuid(
                            request.submission_id.as_uuid(),
                        ),
                        room_id: request.room_id,
                        media_type: request.media_type,
                        plaintext: request.plaintext,
                    })
                    .map_err(ProtectMessageBodyFailure::cryptography)?;
                MessageBody::client_encrypted(
                    protected.ciphertext,
                    request.media_type.clone(),
                    protected.encryption,
                    request.expires_at,
                )
                .map_err(ProtectMessageBodyFailure::invalid_body)
            }
        }
    }
}
