use std::fmt;

use agent_room_domain::{
    DomainError, DomainResult,
    content::{
        ContentByteLength, ContentLifecycleState, ContentMediaType, ContentObject, Sha256Digest,
    },
    ids::{ContentId, ContentUploadRequestId, PrincipalId},
    time::UtcMillis,
};

use super::{ContentByteStream, MatrixEventId, MatrixRoomId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentAccessMode {
    RoomMember,
    SenderOnly,
    Moderator,
}

impl ContentAccessMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RoomMember => "room_member",
            Self::SenderOnly => "sender_only",
            Self::Moderator => "moderator",
        }
    }
}

impl TryFrom<&str> for ContentAccessMode {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "room_member" => Ok(Self::RoomMember),
            "sender_only" => Ok(Self::SenderOnly),
            "moderator" => Ok(Self::Moderator),
            _ => Err(DomainError::Validation {
                field: "content_access_mode",
                reason: "不是支持的访问模式",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAccessPolicy {
    content_id: ContentId,
    matrix_room_id: MatrixRoomId,
    matrix_event_id: Option<MatrixEventId>,
    access_mode: ContentAccessMode,
    created_at: UtcMillis,
    revoked_at: Option<UtcMillis>,
}

impl ContentAccessPolicy {
    pub const fn new(
        content_id: ContentId,
        matrix_room_id: MatrixRoomId,
        access_mode: ContentAccessMode,
        created_at: UtcMillis,
    ) -> Self {
        Self {
            content_id,
            matrix_room_id,
            matrix_event_id: None,
            access_mode,
            created_at,
            revoked_at: None,
        }
    }

    /// 从持久化存储恢复访问策略。
    ///
    /// # Errors
    ///
    /// 撤销时间早于创建时间时返回错误。
    pub fn restore(
        content_id: ContentId,
        matrix_room_id: MatrixRoomId,
        matrix_event_id: Option<MatrixEventId>,
        access_mode: ContentAccessMode,
        created_at: UtcMillis,
        revoked_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        if revoked_at.is_some_and(|revoked_at| revoked_at < created_at) {
            return Err(DomainError::Validation {
                field: "content_policy_revoked_at",
                reason: "不能早于创建时间",
            });
        }
        Ok(Self {
            content_id,
            matrix_room_id,
            matrix_event_id,
            access_mode,
            created_at,
            revoked_at,
        })
    }

    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    pub const fn matrix_room_id(&self) -> &MatrixRoomId {
        &self.matrix_room_id
    }

    pub const fn matrix_event_id(&self) -> Option<&MatrixEventId> {
        self.matrix_event_id.as_ref()
    }

    pub const fn access_mode(&self) -> ContentAccessMode {
        self.access_mode
    }

    pub const fn created_at(&self) -> UtcMillis {
        self.created_at
    }

    pub const fn revoked_at(&self) -> Option<UtcMillis> {
        self.revoked_at
    }

    pub const fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentUploadFingerprint([u8; 32]);

impl ContentUploadFingerprint {
    pub const fn from_bytes(value: [u8; 32]) -> Self {
        Self(value)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentUploadClaim {
    pub request_id: ContentUploadRequestId,
    pub fingerprint: ContentUploadFingerprint,
    pub content: ContentObject,
    pub access_policy: ContentAccessPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentUploadClaimOutcome {
    Created {
        content: ContentObject,
        access_policy: ContentAccessPolicy,
    },
    Existing {
        content: ContentObject,
        access_policy: ContentAccessPolicy,
    },
}

impl ContentUploadClaimOutcome {
    pub const fn content(&self) -> &ContentObject {
        match self {
            Self::Created { content, .. } | Self::Existing { content, .. } => content,
        }
    }

    pub const fn access_policy(&self) -> &ContentAccessPolicy {
        match self {
            Self::Created { access_policy, .. } | Self::Existing { access_policy, .. } => {
                access_policy
            }
        }
    }

    pub const fn was_created(&self) -> bool {
        matches!(self, Self::Created { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentLifecycleTransition {
    pub content_id: ContentId,
    pub expected: ContentLifecycleState,
    pub target: ContentLifecycleState,
    pub changed_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentEventBinding {
    pub content_id: ContentId,
    pub matrix_room_id: MatrixRoomId,
    pub matrix_event_id: MatrixEventId,
    pub bound_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimableContentQuery {
    pub now: UtcMillis,
    pub orphaned_before: UtcMillis,
    pub limit: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectWriteReceipt {
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
}

pub struct OpenedContentObject {
    pub reported_digest: Option<Sha256Digest>,
    pub reported_byte_length: Option<ContentByteLength>,
    pub body: ContentByteStream,
}

impl fmt::Debug for OpenedContentObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenedContentObject")
            .field("reported_digest", &self.reported_digest)
            .field("reported_byte_length", &self.reported_byte_length)
            .field("body", &"[分块内容流]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAuthorizationRequest {
    pub principal_id: PrincipalId,
    pub owner_principal_id: PrincipalId,
    pub matrix_room_id: MatrixRoomId,
    pub access_mode: ContentAccessMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentAuthorizationDecision {
    Allowed,
    Denied,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ContentReadTicket(String);

impl ContentReadTicket {
    /// 接受来自不可信客户端的短期票据字符串。
    ///
    /// # Errors
    ///
    /// 空值、控制字符和超长票据会被拒绝。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > 8_192 || value.chars().any(char::is_control) {
            return Err(DomainError::Validation {
                field: "content_read_ticket",
                reason: "格式或长度无效",
            });
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ContentReadTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[短期内容读取票据]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentReadTicketClaims {
    pub principal_id: PrincipalId,
    pub content_id: ContentId,
    pub matrix_room_id: MatrixRoomId,
    pub matrix_event_id: MatrixEventId,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
    pub issued_at: UtcMillis,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDownloadAttempt {
    pub principal_id: PrincipalId,
    pub content_id: ContentId,
    pub matrix_room_id: MatrixRoomId,
    pub byte_length: ContentByteLength,
    pub attempted_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRateLimitDecision {
    Allowed,
    RetryAt(UtcMillis),
}
