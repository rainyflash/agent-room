use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentStreamFailureKind {
    Source,
    SizeLimitExceeded,
    IntegrityMismatch,
    StorageUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("内容流操作 {operation} 失败：{kind:?}")]
pub struct ContentStreamFailure {
    operation: &'static str,
    kind: ContentStreamFailureKind,
}

impl ContentStreamFailure {
    pub const fn new(operation: &'static str, kind: ContentStreamFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub const fn kind(&self) -> ContentStreamFailureKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectStoreFailureKind {
    NotFound,
    Rejected,
    CorruptMetadata,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("私有对象存储操作 {operation} 失败：{kind:?}")]
pub struct ObjectStoreFailure {
    operation: &'static str,
    kind: ObjectStoreFailureKind,
}

impl ObjectStoreFailure {
    pub const fn new(operation: &'static str, kind: ObjectStoreFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub const fn kind(&self) -> ObjectStoreFailureKind {
        self.kind
    }
}

pub type ObjectStoreResult<T> = Result<T, ObjectStoreFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentAuthorizationFailureKind {
    Denied,
    StaleProjection,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("内容授权操作 {operation} 失败：{kind:?}")]
pub struct ContentAuthorizationFailure {
    operation: &'static str,
    kind: ContentAuthorizationFailureKind,
}

impl ContentAuthorizationFailure {
    pub const fn new(operation: &'static str, kind: ContentAuthorizationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub const fn kind(&self) -> ContentAuthorizationFailureKind {
        self.kind
    }
}

pub type ContentAuthorizationResult<T> = Result<T, ContentAuthorizationFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTicketFailureKind {
    Invalid,
    Expired,
    AudienceMismatch,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("内容票据操作 {operation} 失败：{kind:?}")]
pub struct ContentTicketFailure {
    operation: &'static str,
    kind: ContentTicketFailureKind,
}

impl ContentTicketFailure {
    pub const fn new(operation: &'static str, kind: ContentTicketFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub const fn kind(&self) -> ContentTicketFailureKind {
        self.kind
    }
}

pub type ContentTicketResult<T> = Result<T, ContentTicketFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRateLimitFailureKind {
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("内容限流操作 {operation} 失败：{kind:?}")]
pub struct ContentRateLimitFailure {
    operation: &'static str,
    kind: ContentRateLimitFailureKind,
}

impl ContentRateLimitFailure {
    pub const fn new(operation: &'static str, kind: ContentRateLimitFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub const fn kind(&self) -> ContentRateLimitFailureKind {
        self.kind
    }
}

pub type ContentRateLimitResult<T> = Result<T, ContentRateLimitFailure>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("无法生成不可猜测的私有对象键")]
pub struct ContentStorageKeyGenerationFailure;

pub type ContentStorageKeyGenerationResult<T> = Result<T, ContentStorageKeyGenerationFailure>;
