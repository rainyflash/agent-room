use std::sync::Arc;

use agent_room_application::ports::{MatrixRoomId, PortFuture};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    ids::ContentId,
};
use sha2::{Digest as _, Sha256};

use super::{
    DecryptMessageContentRequest, MessageContentCipher, MessageContentCryptographyFailure,
    MessageContentCryptographyFailureKind, MessageContentSourceQuery, MessageTimelineQueryFailure,
    MessageTimelineQueryRepository, ProjectedMessagePreview,
};

const MAX_INLINE_CONTENT_BYTES: u64 = 48 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContentReadRequest {
    content_id: ContentId,
    maximum_bytes: u64,
}

impl MessageContentReadRequest {
    pub const fn new(content_id: ContentId, maximum_bytes: u64) -> Self {
        Self {
            content_id,
            maximum_bytes,
        }
    }

    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    pub const fn maximum_bytes(&self) -> u64 {
        self.maximum_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedMessageContent {
    pub bytes: Arc<[u8]>,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageContentReadFailureKind {
    InvalidRequest,
    NotFound,
    Denied,
    RateLimited,
    Unavailable,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageContentReadFailure {
    kind: MessageContentReadFailureKind,
}

impl MessageContentReadFailure {
    pub const fn new(kind: MessageContentReadFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> MessageContentReadFailureKind {
        self.kind
    }
}

pub trait MessageContentReadGateway: Send + Sync {
    fn open<'a>(
        &'a self,
        request: &'a MessageContentReadRequest,
    ) -> PortFuture<'a, Result<DownloadedMessageContent, MessageContentReadFailure>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenMessageContentRequest {
    room_id: MatrixRoomId,
    content_id: ContentId,
}

impl OpenMessageContentRequest {
    pub const fn new(room_id: MatrixRoomId, content_id: ContentId) -> Self {
        Self {
            room_id,
            content_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedMessageContent {
    source: ProjectedMessagePreview,
    body: String,
}

impl OpenedMessageContent {
    pub const fn source(&self) -> &ProjectedMessagePreview {
        &self.source
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMessageContentFailureKind {
    Projection,
    NotFound,
    UnsupportedMediaType,
    TooLarge,
    Content,
    IntegrityMismatch,
    InvalidEncoding,
    Cryptography,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenMessageContentFailure {
    kind: OpenMessageContentFailureKind,
    projection: Option<MessageTimelineQueryFailure>,
    content: Option<MessageContentReadFailure>,
    cryptography: Option<MessageContentCryptographyFailure>,
}

impl OpenMessageContentFailure {
    const fn simple(kind: OpenMessageContentFailureKind) -> Self {
        Self {
            kind,
            projection: None,
            content: None,
            cryptography: None,
        }
    }

    const fn projection(failure: MessageTimelineQueryFailure) -> Self {
        Self {
            kind: OpenMessageContentFailureKind::Projection,
            projection: Some(failure),
            content: None,
            cryptography: None,
        }
    }

    const fn content(failure: MessageContentReadFailure) -> Self {
        Self {
            kind: OpenMessageContentFailureKind::Content,
            projection: None,
            content: Some(failure),
            cryptography: None,
        }
    }

    const fn cryptography(failure: MessageContentCryptographyFailure) -> Self {
        Self {
            kind: OpenMessageContentFailureKind::Cryptography,
            projection: None,
            content: None,
            cryptography: Some(failure),
        }
    }

    pub const fn kind(self) -> OpenMessageContentFailureKind {
        self.kind
    }

    pub const fn projection_failure(self) -> Option<MessageTimelineQueryFailure> {
        self.projection
    }

    pub const fn content_failure(self) -> Option<MessageContentReadFailure> {
        self.content
    }

    pub const fn cryptography_failure(self) -> Option<MessageContentCryptographyFailure> {
        self.cryptography
    }
}

pub struct OpenMessageContentDependencies {
    pub projections: Arc<dyn MessageTimelineQueryRepository>,
    pub content: Arc<dyn MessageContentReadGateway>,
    pub cryptography: Option<Arc<dyn MessageContentCipher>>,
}

pub struct OpenMessageContentService {
    projections: Arc<dyn MessageTimelineQueryRepository>,
    content: Arc<dyn MessageContentReadGateway>,
    cryptography: Option<Arc<dyn MessageContentCipher>>,
}

impl OpenMessageContentService {
    pub fn new(dependencies: OpenMessageContentDependencies) -> Self {
        Self {
            projections: dependencies.projections,
            content: dependencies.content,
            cryptography: dependencies.cryptography,
        }
    }

    /// 从本地已验签投影定位正文，显式下载后再次校验媒体类型、长度和摘要。
    ///
    /// # Errors
    ///
    /// 来源不存在、正文不适合 IPC、远端读取失败或完整性不一致时返回稳定错误。
    pub async fn open(
        &self,
        request: &OpenMessageContentRequest,
    ) -> Result<OpenedMessageContent, OpenMessageContentFailure> {
        let source = self
            .projections
            .find_content_source(&MessageContentSourceQuery::new(
                request.room_id.clone(),
                request.content_id,
            ))
            .await
            .map_err(OpenMessageContentFailure::projection)?
            .ok_or_else(|| {
                OpenMessageContentFailure::simple(OpenMessageContentFailureKind::NotFound)
            })?;
        let expected = &source.content;
        if !is_inline_text(source.preview.content_type()) {
            return Err(OpenMessageContentFailure::simple(
                OpenMessageContentFailureKind::UnsupportedMediaType,
            ));
        }
        let plaintext_size = expected
            .client_encryption()
            .map_or(expected.size_bytes(), |encryption| {
                encryption.plaintext_size_bytes()
            });
        if plaintext_size > MAX_INLINE_CONTENT_BYTES {
            return Err(OpenMessageContentFailure::simple(
                OpenMessageContentFailureKind::TooLarge,
            ));
        }
        let opened = self
            .content
            .open(&MessageContentReadRequest::new(
                request.content_id,
                expected.size_bytes(),
            ))
            .await
            .map_err(OpenMessageContentFailure::content)?;
        if !content_matches(&source, &opened) {
            return Err(OpenMessageContentFailure::simple(
                OpenMessageContentFailureKind::IntegrityMismatch,
            ));
        }
        let plaintext = match expected.client_encryption() {
            Some(encryption) => self
                .cryptography
                .as_ref()
                .ok_or_else(|| {
                    OpenMessageContentFailure::cryptography(MessageContentCryptographyFailure::new(
                        MessageContentCryptographyFailureKind::Unavailable,
                    ))
                })?
                .decrypt(&DecryptMessageContentRequest {
                    room_id: &source.room_id,
                    media_type: source.preview.content_type(),
                    ciphertext: &opened.bytes,
                    encryption,
                })
                .map_err(OpenMessageContentFailure::cryptography)?,
            None => opened.bytes.clone(),
        };
        let body = std::str::from_utf8(&plaintext)
            .map_err(|_| {
                OpenMessageContentFailure::simple(OpenMessageContentFailureKind::InvalidEncoding)
            })?
            .to_owned();
        Ok(OpenedMessageContent { source, body })
    }
}

fn is_inline_text(media_type: &ContentMediaType) -> bool {
    matches!(
        media_type.as_str(),
        "application/json" | "text/markdown" | "text/plain"
    )
}

fn content_matches(source: &ProjectedMessagePreview, opened: &DownloadedMessageContent) -> bool {
    let expected = &source.content;
    let actual_length = u64::try_from(opened.bytes.len()).ok();
    let actual_digest = Sha256Digest::from_bytes(Sha256::digest(&opened.bytes).into());
    opened.media_type == *source.preview.content_type()
        && opened.byte_length.value() == expected.size_bytes()
        && actual_length == Some(expected.size_bytes())
        && opened.digest == expected.digest()
        && actual_digest == expected.digest()
}
