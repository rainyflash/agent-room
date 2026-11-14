use agent_room_domain::DomainError;

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{MatrixFailure, MatrixFailureKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomFailureStage {
    Validation,
    Directory,
    Matrix,
    Persistence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomFailureKind {
    InvalidRequest,
    Forbidden,
    /// 管得了这间房，但这件事只有房主能做：设、撤管理员，或者动一位管理员
    /// （`specs/redesign/room-roles.md`）。
    OwnerOnly,
    /// 做得了，但要最近登录过：设、撤管理员，请一位管理员进来（`specs/redesign/room-roles.md` R2）。
    /// 和别的要最近登录的操作回同一个码，网页据此请人重新登录。
    ReauthenticationRequired,
    NotFound,
    Conflict,
    DependencyUnavailable,
    UnknownCommit,
    /// 新建时就公开，可房主名下已经有一间公开房间（每个账户最多 1 间）。
    PublicRoomLimitReached,
    /// 子房间这边不让做，分开说原因（`specs/redesign/sub-rooms.md`）。
    SubRoom(SubRoomRefusal),
    Internal,
}

/// 子房间这边不让做的几种原因，回答里各有各的码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomRefusal {
    /// 这件事子房间没有，只在父房间做：口令、邀请链接、公开、往外请人、转让房主、单独改权限，
    /// 还有整间归档和改名（子房间有自己的入口）。
    ParentOnly,
    /// 子房间下面不能再开子房间。
    Nested,
    /// 房主定了只有管理员能建子房间。
    AdminsOnly,
    /// 房主关掉了“Agent 能建子房间”。
    AgentsMayNotCreate,
    /// 这间房里已经有一间没归档的子房间叫这个名字。
    NameTaken,
    /// 这间房里没归档的子房间到上限了。
    LimitReached,
    /// 这个人或者这个 Agent 这一小时建得太多。
    RateLimited,
    /// 谁能建子房间这几项设置只有房主能改。
    PolicyOwnerOnly,
    /// 这间子房间（或者它所在的房间）归档了：进不去，也改不了。
    Archived,
    /// 他在这间子房间里被封禁着。
    Banned,
    /// 这间要批准：得先申请。
    ApprovalRequired,
    /// 这间只能请进来。
    InviteOnly,
    /// 房主在每一间子房间里，退不出去。
    OwnerStays,
    /// 改名、改用途要管得了这一间：这一间的管理员，或者房间的房主和管理员。
    ManagersOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateRoomFailure {
    operation: &'static str,
    stage: PrivateRoomFailureStage,
    kind: PrivateRoomFailureKind,
    /// 请求里哪一项没过校验（固定的字段名，比如 `room_name`），只给日志看。
    rejected: Option<&'static str>,
}

impl PrivateRoomFailure {
    pub const fn new(
        operation: &'static str,
        stage: PrivateRoomFailureStage,
        kind: PrivateRoomFailureKind,
    ) -> Self {
        Self {
            operation,
            stage,
            kind,
            rejected: None,
        }
    }

    #[must_use]
    pub const fn rejecting(mut self, field: &'static str) -> Self {
        self.rejected = Some(field);
        self
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn stage(self) -> PrivateRoomFailureStage {
        self.stage
    }

    pub const fn kind(self) -> PrivateRoomFailureKind {
        self.kind
    }

    pub const fn rejected(self) -> Option<&'static str> {
        self.rejected
    }
}

pub type PrivateRoomResult<T> = Result<T, PrivateRoomFailure>;

pub(crate) const fn domain(operation: &'static str, error: &DomainError) -> PrivateRoomFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => PrivateRoomFailureKind::Forbidden,
        DomainError::OwnerOnly { .. } => PrivateRoomFailureKind::OwnerOnly,
        DomainError::Validation { field, .. } => {
            return PrivateRoomFailure::new(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::InvalidRequest,
            )
            .rejecting(field);
        }
        DomainError::InvalidTransition { .. } => PrivateRoomFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => PrivateRoomFailureKind::Internal,
    };
    PrivateRoomFailure::new(operation, PrivateRoomFailureStage::Validation, kind)
}

pub(crate) const fn repository(
    operation: &'static str,
    stage: PrivateRoomFailureStage,
    error: &RepositoryError,
) -> PrivateRoomFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => PrivateRoomFailureKind::Conflict,
        RepositoryErrorKind::Constraint => PrivateRoomFailureKind::InvalidRequest,
        RepositoryErrorKind::Forbidden => PrivateRoomFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => PrivateRoomFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => PrivateRoomFailureKind::DependencyUnavailable,
        RepositoryErrorKind::CorruptData => PrivateRoomFailureKind::Internal,
    };
    PrivateRoomFailure::new(operation, stage, kind)
}

pub(crate) const fn matrix(operation: &'static str, error: MatrixFailure) -> PrivateRoomFailure {
    let kind = match error.kind() {
        MatrixFailureKind::Conflict => PrivateRoomFailureKind::Conflict,
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable => PrivateRoomFailureKind::DependencyUnavailable,
        MatrixFailureKind::UnknownCommit => PrivateRoomFailureKind::UnknownCommit,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => PrivateRoomFailureKind::Internal,
    };
    PrivateRoomFailure::new(operation, PrivateRoomFailureStage::Matrix, kind)
}

pub(crate) const fn failure(
    operation: &'static str,
    stage: PrivateRoomFailureStage,
    kind: PrivateRoomFailureKind,
) -> PrivateRoomFailure {
    PrivateRoomFailure::new(operation, stage, kind)
}
