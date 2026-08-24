use std::sync::Arc;

use agent_room_application::ports::MatrixRoomId;
use agent_room_domain::{
    content::{ContentByteLength, ContentEncryptionMode, ContentMediaType, Sha256Digest},
    ids::{ContentId, MessageId, MessageSubmissionId},
    messages::{MessagePreview, MessageProvenance, MessageRelation},
    time::UtcMillis,
};
use sha2::{Digest, Sha256};
use uuid::Version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBody {
    bytes: Arc<[u8]>,
    digest: Sha256Digest,
    byte_length: ContentByteLength,
    media_type: ContentMediaType,
    encryption_mode: ContentEncryptionMode,
    expires_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRequestError {
    InvalidSubmissionId,
    InvalidBodyLength,
    MediaTypeMismatch,
}

impl MessageBody {
    /// 创建待上传的不可变消息正文。
    ///
    /// # Errors
    ///
    /// 正文为空或超过内容服务上限时返回错误。
    pub fn new(
        bytes: impl Into<Arc<[u8]>>,
        media_type: ContentMediaType,
        encryption_mode: ContentEncryptionMode,
        expires_at: Option<UtcMillis>,
    ) -> Result<Self, MessageRequestError> {
        let bytes = bytes.into();
        let byte_length = u64::try_from(bytes.len())
            .ok()
            .and_then(|length| ContentByteLength::new(length).ok())
            .ok_or(MessageRequestError::InvalidBodyLength)?;
        let digest = Sha256Digest::from_bytes(Sha256::digest(&bytes).into());
        Ok(Self {
            bytes,
            digest,
            byte_length,
            media_type,
            encryption_mode,
            expires_at,
        })
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    pub const fn byte_length(&self) -> ContentByteLength {
        self.byte_length
    }

    pub const fn media_type(&self) -> &ContentMediaType {
        &self.media_type
    }

    pub const fn encryption_mode(&self) -> ContentEncryptionMode {
        self.encryption_mode
    }

    pub const fn expires_at(&self) -> Option<UtcMillis> {
        self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessageRequest {
    submission_id: MessageSubmissionId,
    room_id: MatrixRoomId,
    preview: MessagePreview,
    body: MessageBody,
    provenance: MessageProvenance,
    relation: Option<MessageRelation>,
}

impl SendMessageRequest {
    /// 创建新消息发送意图。
    ///
    /// # Errors
    ///
    /// 幂等标识不是 `UUIDv7`，或预览媒体类型与正文不一致时返回错误。
    pub fn new(
        submission_id: MessageSubmissionId,
        room_id: MatrixRoomId,
        preview: MessagePreview,
        body: MessageBody,
        provenance: MessageProvenance,
        relation: Option<MessageRelation>,
    ) -> Result<Self, MessageRequestError> {
        validate_submission(submission_id)?;
        if preview.content_type() != body.media_type() {
            return Err(MessageRequestError::MediaTypeMismatch);
        }
        Ok(Self {
            submission_id,
            room_id,
            preview,
            body,
            provenance,
            relation,
        })
    }

    pub const fn submission_id(&self) -> MessageSubmissionId {
        self.submission_id
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn preview(&self) -> &MessagePreview {
        &self.preview
    }

    pub const fn body(&self) -> &MessageBody {
        &self.body
    }

    pub const fn provenance(&self) -> MessageProvenance {
        self.provenance
    }

    pub const fn relation(&self) -> Option<MessageRelation> {
        self.relation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditMessageRequest {
    submission_id: MessageSubmissionId,
    room_id: MatrixRoomId,
    target_message_id: MessageId,
    preview: MessagePreview,
    body: MessageBody,
    provenance: MessageProvenance,
}

impl EditMessageRequest {
    /// 创建替换消息正文和预览的修订意图。
    ///
    /// # Errors
    ///
    /// 幂等标识不是 `UUIDv7`，或预览媒体类型与正文不一致时返回错误。
    pub fn new(
        submission_id: MessageSubmissionId,
        room_id: MatrixRoomId,
        target_message_id: MessageId,
        preview: MessagePreview,
        body: MessageBody,
        provenance: MessageProvenance,
    ) -> Result<Self, MessageRequestError> {
        validate_submission(submission_id)?;
        if preview.content_type() != body.media_type() {
            return Err(MessageRequestError::MediaTypeMismatch);
        }
        Ok(Self {
            submission_id,
            room_id,
            target_message_id,
            preview,
            body,
            provenance,
        })
    }

    pub const fn submission_id(&self) -> MessageSubmissionId {
        self.submission_id
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn target_message_id(&self) -> MessageId {
        self.target_message_id
    }

    pub const fn preview(&self) -> &MessagePreview {
        &self.preview
    }

    pub const fn body(&self) -> &MessageBody {
        &self.body
    }

    pub const fn provenance(&self) -> MessageProvenance {
        self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactMessageRequest {
    submission_id: MessageSubmissionId,
    room_id: MatrixRoomId,
    target_message_id: MessageId,
    target_content_id: ContentId,
    provenance: MessageProvenance,
}

impl RedactMessageRequest {
    /// 创建先撤销正文读取权、再发布时间线修订的意图。
    ///
    /// # Errors
    ///
    /// 幂等标识不是 `UUIDv7` 时返回错误。
    pub fn new(
        submission_id: MessageSubmissionId,
        room_id: MatrixRoomId,
        target_message_id: MessageId,
        target_content_id: ContentId,
        provenance: MessageProvenance,
    ) -> Result<Self, MessageRequestError> {
        validate_submission(submission_id)?;
        Ok(Self {
            submission_id,
            room_id,
            target_message_id,
            target_content_id,
            provenance,
        })
    }

    pub const fn submission_id(&self) -> MessageSubmissionId {
        self.submission_id
    }

    pub const fn room_id(&self) -> &MatrixRoomId {
        &self.room_id
    }

    pub const fn target_message_id(&self) -> MessageId {
        self.target_message_id
    }

    pub const fn target_content_id(&self) -> ContentId {
        self.target_content_id
    }

    pub const fn provenance(&self) -> MessageProvenance {
        self.provenance
    }
}

fn validate_submission(submission_id: MessageSubmissionId) -> Result<(), MessageRequestError> {
    if submission_id.as_uuid().get_version() == Some(Version::SortRand) {
        Ok(())
    } else {
        Err(MessageRequestError::InvalidSubmissionId)
    }
}
