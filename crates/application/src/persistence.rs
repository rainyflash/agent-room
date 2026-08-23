use thiserror::Error;

pub type RepositoryResult<T> = Result<T, RepositoryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryErrorKind {
    Conflict,
    Constraint,
    NotFound,
    Unavailable,
    CorruptData,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("持久化操作 {operation} 失败：{kind:?}")]
pub struct RepositoryError {
    operation: &'static str,
    kind: RepositoryErrorKind,
}

impl RepositoryError {
    pub const fn new(operation: &'static str, kind: RepositoryErrorKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    pub const fn kind(&self) -> RepositoryErrorKind {
        self.kind
    }
}
