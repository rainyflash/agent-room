use agent_room_domain::policy::AutomationGrantDenial;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationFailureKind {
    InvalidRequest,
    Forbidden,
    NotFound,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationFailure {
    operation: &'static str,
    kind: AutomationFailureKind,
}

impl AutomationFailure {
    pub(crate) const fn new(operation: &'static str, kind: AutomationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AutomationFailureKind {
        self.kind
    }
}

pub type AutomationResult<T> = Result<T, AutomationFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationSendDenial {
    GrantNotFound,
    Grant(AutomationGrantDenial),
    ActorMismatch,
    AuthorityChanged,
    MatrixPermissionDenied,
}

impl AutomationSendDenial {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GrantNotFound => "automation.grant_not_found",
            Self::Grant(reason) => reason.as_str(),
            Self::ActorMismatch => "automation.actor_mismatch",
            Self::AuthorityChanged => "automation.authority_changed",
            Self::MatrixPermissionDenied => "automation.matrix_permission_denied",
        }
    }
}
