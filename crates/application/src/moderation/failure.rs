use agent_room_domain::time::UtcMillis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationFailureKind {
    InvalidRequest,
    Forbidden,
    /// 管得了这间房，但对象是管理员：只有房主能动（`specs/redesign/room-roles.md`）。
    OwnerOnly,
    NotFound,
    Conflict,
    RateLimited,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationFailure {
    operation: &'static str,
    kind: ModerationFailureKind,
    retry_at: Option<UtcMillis>,
}

impl ModerationFailure {
    pub const fn new(operation: &'static str, kind: ModerationFailureKind) -> Self {
        Self {
            operation,
            kind,
            retry_at: None,
        }
    }

    pub(crate) const fn rate_limited(operation: &'static str, retry_at: UtcMillis) -> Self {
        Self {
            operation,
            kind: ModerationFailureKind::RateLimited,
            retry_at: Some(retry_at),
        }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> ModerationFailureKind {
        self.kind
    }

    pub const fn retry_at(self) -> Option<UtcMillis> {
        self.retry_at
    }
}

pub type ModerationResult<T> = Result<T, ModerationFailure>;
