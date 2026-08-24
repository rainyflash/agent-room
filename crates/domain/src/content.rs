use crate::{
    DomainError, DomainResult,
    ids::{ContentId, PrincipalId},
    time::UtcMillis,
};

pub const MAX_CONTENT_BYTES: u64 = 25 * 1024 * 1024;

const SUPPORTED_MEDIA_TYPES: [&str; 9] = [
    "application/json",
    "application/octet-stream",
    "application/pdf",
    "image/gif",
    "image/jpeg",
    "image/png",
    "image/webp",
    "text/markdown",
    "text/plain",
];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentByteLength(u64);

impl ContentByteLength {
    /// 创建受服务端硬上限约束的内容长度。
    ///
    /// # Errors
    ///
    /// 空内容或超过 25 MiB 的内容会被拒绝。
    pub fn new(value: u64) -> DomainResult<Self> {
        if value == 0 || value > MAX_CONTENT_BYTES {
            return Err(DomainError::Validation {
                field: "content_byte_length",
                reason: "必须在 1 字节到 25 MiB 之间",
            });
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentMediaType(String);

impl ContentMediaType {
    /// 创建白名单内的规范媒体类型。
    ///
    /// # Errors
    ///
    /// 参数、大小写变体和未支持的类型会被拒绝，避免声明与实际解析策略分叉。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        if SUPPORTED_MEDIA_TYPES
            .binary_search(&value.as_str())
            .is_err()
        {
            return Err(DomainError::Validation {
                field: "content_media_type",
                reason: "不在允许的媒体类型白名单中",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentStorageKey(String);

impl ContentStorageKey {
    /// 校验服务端生成的私有对象键。
    ///
    /// # Errors
    ///
    /// 过短、绝对路径、路径穿越或包含不安全字符的对象键会被拒绝。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let valid_characters = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'));
        if !(16..=1024).contains(&value.len())
            || value.starts_with('/')
            || value.ends_with('/')
            || value
                .split('/')
                .any(|segment| segment.is_empty() || segment == "..")
            || !valid_characters
        {
            return Err(DomainError::Validation {
                field: "content_storage_key",
                reason: "必须是服务端生成的安全相对对象键",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentEncryptionMode {
    ServerSide,
    ClientE2ee,
}

impl ContentEncryptionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ServerSide => "server_side",
            Self::ClientE2ee => "client_e2ee",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentScanState {
    Pending,
    Clean,
    Suspicious,
    Rejected,
    NotApplicable,
}

impl ContentScanState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Clean => "clean",
            Self::Suspicious => "suspicious",
            Self::Rejected => "rejected",
            Self::NotApplicable => "not_applicable",
        }
    }

    pub const fn allows_read(self) -> bool {
        matches!(self, Self::Clean | Self::NotApplicable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentLifecycleState {
    Uploading,
    Active,
    Orphaned,
    Redacted,
    Expired,
    Deleted,
}

impl ContentLifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Active => "active",
            Self::Orphaned => "orphaned",
            Self::Redacted => "redacted",
            Self::Expired => "expired",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentObjectFields {
    pub id: ContentId,
    pub owner_principal_id: PrincipalId,
    pub storage_key: ContentStorageKey,
    pub digest: Sha256Digest,
    pub byte_length: ContentByteLength,
    pub media_type: ContentMediaType,
    pub encryption_mode: ContentEncryptionMode,
    pub scan_state: ContentScanState,
    pub lifecycle_state: ContentLifecycleState,
    pub expires_at: Option<UtcMillis>,
    pub created_at: UtcMillis,
    pub deleted_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentObject {
    fields: ContentObjectFields,
}

impl ContentObject {
    /// 创建等待写入私有对象存储的内容记录。
    ///
    /// # Errors
    ///
    /// 到期时间非法或客户端密文仍声明需要服务端扫描时会被拒绝。
    pub fn begin_upload(mut fields: ContentObjectFields) -> DomainResult<Self> {
        fields.lifecycle_state = ContentLifecycleState::Uploading;
        fields.deleted_at = None;
        Self::restore(fields)
    }

    /// 从权威存储恢复内容聚合，并重新验证跨字段不变式。
    ///
    /// # Errors
    ///
    /// 持久化数据违反生命周期、删除时间、到期时间或加密扫描约束时返回错误。
    pub fn restore(fields: ContentObjectFields) -> DomainResult<Self> {
        validate_fields(&fields)?;
        Ok(Self { fields })
    }

    pub const fn id(&self) -> ContentId {
        self.fields.id
    }

    pub const fn owner_principal_id(&self) -> PrincipalId {
        self.fields.owner_principal_id
    }

    pub const fn storage_key(&self) -> &ContentStorageKey {
        &self.fields.storage_key
    }

    pub const fn digest(&self) -> Sha256Digest {
        self.fields.digest
    }

    pub const fn byte_length(&self) -> ContentByteLength {
        self.fields.byte_length
    }

    pub const fn media_type(&self) -> &ContentMediaType {
        &self.fields.media_type
    }

    pub const fn encryption_mode(&self) -> ContentEncryptionMode {
        self.fields.encryption_mode
    }

    pub const fn scan_state(&self) -> ContentScanState {
        self.fields.scan_state
    }

    pub const fn lifecycle_state(&self) -> ContentLifecycleState {
        self.fields.lifecycle_state
    }

    pub const fn expires_at(&self) -> Option<UtcMillis> {
        self.fields.expires_at
    }

    pub const fn created_at(&self) -> UtcMillis {
        self.fields.created_at
    }

    pub const fn deleted_at(&self) -> Option<UtcMillis> {
        self.fields.deleted_at
    }

    /// 记录隔离扫描结果；终态结果不可被后续扫描覆盖。
    ///
    /// # Errors
    ///
    /// 客户端密文、已产生终态扫描结果或已删除对象不能再次写入扫描结果。
    pub fn record_scan(&mut self, outcome: ContentScanState) -> DomainResult<()> {
        if self.fields.encryption_mode == ContentEncryptionMode::ClientE2ee
            || self.fields.scan_state != ContentScanState::Pending
            || self.fields.lifecycle_state == ContentLifecycleState::Deleted
            || outcome == ContentScanState::Pending
            || outcome == ContentScanState::NotApplicable
        {
            return Err(DomainError::InvalidTransition {
                entity: "content_scan",
                from: self.fields.scan_state.as_str(),
                to: outcome.as_str(),
            });
        }
        self.fields.scan_state = outcome;
        Ok(())
    }

    /// 在字节摘要和扫描状态均验证后激活内容，重复调用保持幂等。
    ///
    /// # Errors
    ///
    /// 未通过扫描或已经进入不可恢复状态的内容不能激活。
    pub fn activate(&mut self) -> DomainResult<()> {
        if !self.fields.scan_state.allows_read() {
            return Err(DomainError::InvariantViolation {
                entity: "content_object",
                rule: "只有通过扫描或客户端加密的内容可以激活",
            });
        }
        transition(
            &mut self.fields.lifecycle_state,
            ContentLifecycleState::Active,
            &[
                ContentLifecycleState::Uploading,
                ContentLifecycleState::Active,
            ],
        )
    }

    /// 将未能绑定 Matrix 事件的对象标记为孤儿，重复调用保持幂等。
    ///
    /// # Errors
    ///
    /// 已撤回、过期或删除的对象不能倒退为孤儿状态。
    pub fn mark_orphaned(&mut self) -> DomainResult<()> {
        transition(
            &mut self.fields.lifecycle_state,
            ContentLifecycleState::Orphaned,
            &[
                ContentLifecycleState::Uploading,
                ContentLifecycleState::Active,
                ContentLifecycleState::Orphaned,
            ],
        )
    }

    /// 撤回内容访问权，重复调用保持幂等。
    ///
    /// # Errors
    ///
    /// 已过期或删除的对象不能改写其终态原因。
    pub fn redact(&mut self) -> DomainResult<()> {
        transition(
            &mut self.fields.lifecycle_state,
            ContentLifecycleState::Redacted,
            &[
                ContentLifecycleState::Uploading,
                ContentLifecycleState::Active,
                ContentLifecycleState::Orphaned,
                ContentLifecycleState::Redacted,
            ],
        )
    }

    /// 在配置的保留期限到达后标记过期，重复调用保持幂等。
    ///
    /// # Errors
    ///
    /// 未配置到期时间、尚未到期或已撤回/删除时返回错误。
    pub fn expire(&mut self, now: UtcMillis) -> DomainResult<()> {
        let Some(expires_at) = self.fields.expires_at else {
            return Err(DomainError::InvariantViolation {
                entity: "content_object",
                rule: "无到期时间的内容不能由保留任务过期",
            });
        };
        if now < expires_at {
            return Err(DomainError::InvariantViolation {
                entity: "content_object",
                rule: "内容尚未到达保留期限",
            });
        }
        transition(
            &mut self.fields.lifecycle_state,
            ContentLifecycleState::Expired,
            &[
                ContentLifecycleState::Uploading,
                ContentLifecycleState::Active,
                ContentLifecycleState::Orphaned,
                ContentLifecycleState::Expired,
            ],
        )
    }

    /// 在对象字节清理成功后进入删除终态。
    ///
    /// # Errors
    ///
    /// 活跃或仍在上传的内容不能绕过撤回、过期或孤儿状态直接删除。
    pub fn mark_deleted(&mut self, deleted_at: UtcMillis) -> DomainResult<()> {
        if self.fields.lifecycle_state == ContentLifecycleState::Deleted {
            return Ok(());
        }
        if deleted_at < self.fields.created_at {
            return Err(DomainError::Validation {
                field: "content_deleted_at",
                reason: "不能早于创建时间",
            });
        }
        transition(
            &mut self.fields.lifecycle_state,
            ContentLifecycleState::Deleted,
            &[
                ContentLifecycleState::Orphaned,
                ContentLifecycleState::Redacted,
                ContentLifecycleState::Expired,
            ],
        )?;
        self.fields.deleted_at = Some(deleted_at);
        Ok(())
    }

    pub fn is_readable_at(&self, now: UtcMillis) -> bool {
        self.fields.lifecycle_state == ContentLifecycleState::Active
            && self.fields.scan_state.allows_read()
            && self
                .fields
                .expires_at
                .is_none_or(|expires_at| now < expires_at)
    }
}

fn validate_fields(fields: &ContentObjectFields) -> DomainResult<()> {
    if fields
        .expires_at
        .is_some_and(|expires_at| expires_at <= fields.created_at)
    {
        return Err(DomainError::Validation {
            field: "content_expires_at",
            reason: "必须晚于创建时间",
        });
    }
    if fields.encryption_mode == ContentEncryptionMode::ClientE2ee
        && fields.scan_state != ContentScanState::NotApplicable
    {
        return Err(DomainError::InvariantViolation {
            entity: "content_object",
            rule: "客户端密文不能声明服务端正文扫描状态",
        });
    }
    if fields.lifecycle_state == ContentLifecycleState::Active && !fields.scan_state.allows_read() {
        return Err(DomainError::InvariantViolation {
            entity: "content_object",
            rule: "活跃内容必须已通过扫描或为客户端密文",
        });
    }
    match (fields.lifecycle_state, fields.deleted_at) {
        (ContentLifecycleState::Deleted, Some(deleted_at)) if deleted_at >= fields.created_at => {}
        (ContentLifecycleState::Deleted, _) => {
            return Err(DomainError::InvariantViolation {
                entity: "content_object",
                rule: "删除终态必须记录合法删除时间",
            });
        }
        (_, None) => {}
        (_, Some(_)) => {
            return Err(DomainError::InvariantViolation {
                entity: "content_object",
                rule: "非删除状态不能携带删除时间",
            });
        }
    }
    Ok(())
}

fn transition(
    current: &mut ContentLifecycleState,
    target: ContentLifecycleState,
    allowed: &[ContentLifecycleState],
) -> DomainResult<()> {
    if !allowed.contains(current) {
        return Err(DomainError::InvalidTransition {
            entity: "content_object",
            from: current.as_str(),
            to: target.as_str(),
        });
    }
    *current = target;
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, MAX_CONTENT_BYTES,
        Sha256Digest,
    };
    use crate::{
        ids::{ContentId, PrincipalId},
        time::UtcMillis,
    };

    #[test]
    fn 内容长度和媒体类型在领域边界被拒绝() {
        assert!(ContentByteLength::new(0).is_err());
        assert!(ContentByteLength::new(MAX_CONTENT_BYTES + 1).is_err());
        assert!(ContentMediaType::new("text/html").is_err());
        assert!(ContentMediaType::new("text/plain; charset=utf-8").is_err());
    }

    #[test]
    fn 对象键拒绝路径穿越和用户输入字符() {
        assert!(ContentStorageKey::new("content/../secret-object").is_err());
        assert!(ContentStorageKey::new("content/用户文件.txt").is_err());
        assert!(ContentStorageKey::new("content/0195-safe-object").is_ok());
    }

    #[test]
    fn 上传激活孤儿和删除形成单向生命周期() {
        let mut content = server_side_content(ContentScanState::Pending, None);
        assert!(content.activate().is_err());

        content
            .record_scan(ContentScanState::Clean)
            .expect("扫描结果有效");
        content.activate().expect("验证后可以激活");
        assert!(content.is_readable_at(time(2_000)));

        content.mark_orphaned().expect("事件失败后成为孤儿");
        assert!(!content.is_readable_at(time(2_000)));
        content.mark_deleted(time(3_000)).expect("孤儿对象可以清理");
        assert_eq!(content.lifecycle_state(), ContentLifecycleState::Deleted);
        assert!(content.activate().is_err());
    }

    #[test]
    fn 权限读取在到期瞬间立即关闭() {
        let mut content = server_side_content(ContentScanState::Clean, Some(time(2_000)));
        content.activate().expect("扫描通过后激活");

        assert!(content.is_readable_at(time(1_999)));
        assert!(!content.is_readable_at(time(2_000)));
        assert!(content.expire(time(1_999)).is_err());
        content.expire(time(2_000)).expect("到期后进入过期态");
    }

    #[test]
    fn 客户端密文不能伪装成已扫描正文() {
        let fields = ContentObjectFields {
            encryption_mode: ContentEncryptionMode::ClientE2ee,
            scan_state: ContentScanState::Clean,
            ..base_fields()
        };
        assert!(ContentObject::begin_upload(fields).is_err());
    }

    fn server_side_content(
        scan_state: ContentScanState,
        expires_at: Option<UtcMillis>,
    ) -> ContentObject {
        let fields = ContentObjectFields {
            scan_state,
            expires_at,
            ..base_fields()
        };
        ContentObject::begin_upload(fields).expect("测试内容有效")
    }

    fn base_fields() -> ContentObjectFields {
        ContentObjectFields {
            id: ContentId::from_uuid(Uuid::now_v7()),
            owner_principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            storage_key: ContentStorageKey::new(format!("content/{}", Uuid::now_v7()))
                .expect("对象键有效"),
            digest: Sha256Digest::from_bytes([7; 32]),
            byte_length: ContentByteLength::new(16).expect("长度有效"),
            media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
            encryption_mode: ContentEncryptionMode::ServerSide,
            scan_state: ContentScanState::Pending,
            lifecycle_state: ContentLifecycleState::Uploading,
            expires_at: None,
            created_at: time(1_000),
            deleted_at: None,
        }
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
