use crate::{DomainError, DomainResult, ids::ContentId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub const fn from_bytes(value: [u8; 32]) -> Self {
        Self(value)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentStatus {
    Pending,
    Stored,
    Expired,
    Redacted,
}

impl ContentStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Stored => "stored",
            Self::Expired => "expired",
            Self::Redacted => "redacted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentObject {
    id: ContentId,
    digest: Sha256Digest,
    size_bytes: u64,
    status: ContentStatus,
}

impl ContentObject {
    /// 创建等待写入对象存储的内容对象。
    ///
    /// # Errors
    ///
    /// 零字节内容不满足当前内容对象约束。
    pub fn pending(id: ContentId, digest: Sha256Digest, size_bytes: u64) -> DomainResult<Self> {
        if size_bytes == 0 {
            return Err(DomainError::Validation {
                field: "size_bytes",
                reason: "必须大于零",
            });
        }

        Ok(Self {
            id,
            digest,
            size_bytes,
            status: ContentStatus::Pending,
        })
    }

    pub const fn id(&self) -> ContentId {
        self.id
    }

    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub const fn status(&self) -> ContentStatus {
        self.status
    }

    /// 标记内容字节已持久化，重复调用保持幂等。
    ///
    /// # Errors
    ///
    /// 已过期或已撤回内容不能恢复为已存储状态。
    pub fn mark_stored(&mut self) -> DomainResult<()> {
        match self.status {
            ContentStatus::Pending | ContentStatus::Stored => {
                self.status = ContentStatus::Stored;
                Ok(())
            }
            ContentStatus::Expired | ContentStatus::Redacted => {
                Err(DomainError::InvalidTransition {
                    entity: "content_object",
                    from: self.status.label(),
                    to: "stored",
                })
            }
        }
    }

    /// 标记已存储内容过期，重复调用保持幂等。
    ///
    /// # Errors
    ///
    /// 等待上传或已撤回内容不能进入过期状态。
    pub fn expire(&mut self) -> DomainResult<()> {
        match self.status {
            ContentStatus::Stored | ContentStatus::Expired => {
                self.status = ContentStatus::Expired;
                Ok(())
            }
            ContentStatus::Pending | ContentStatus::Redacted => {
                Err(DomainError::InvalidTransition {
                    entity: "content_object",
                    from: self.status.label(),
                    to: "expired",
                })
            }
        }
    }

    pub fn redact(&mut self) {
        self.status = ContentStatus::Redacted;
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{ContentObject, ContentStatus, Sha256Digest};
    use crate::ids::ContentId;

    #[test]
    fn 内容状态不会从终态复活() {
        let mut content = ContentObject::pending(
            ContentId::from_uuid(Uuid::from_u128(1)),
            Sha256Digest::from_bytes([7; 32]),
            16,
        )
        .expect("内容有效");

        content.mark_stored().expect("存储完成");
        content.expire().expect("允许过期");
        assert_eq!(content.status(), ContentStatus::Expired);
        assert!(content.mark_stored().is_err());
    }
}
