use agent_room_domain::DomainError;

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{MatrixFailure, MatrixFailureKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectSessionFailureStage {
    Validation,
    Directory,
    Matrix,
    Persistence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectSessionFailureKind {
    InvalidRequest,
    Forbidden,
    Blocked,
    NotFound,
    Conflict,
    DependencyUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectSessionFailure {
    operation: &'static str,
    stage: DirectSessionFailureStage,
    kind: DirectSessionFailureKind,
}

impl DirectSessionFailure {
    pub(crate) const fn new(
        operation: &'static str,
        stage: DirectSessionFailureStage,
        kind: DirectSessionFailureKind,
    ) -> Self {
        Self {
            operation,
            stage,
            kind,
        }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn stage(self) -> DirectSessionFailureStage {
        self.stage
    }

    pub const fn kind(self) -> DirectSessionFailureKind {
        self.kind
    }
}

pub type DirectSessionResult<T> = Result<T, DirectSessionFailure>;

pub(crate) const fn domain(operation: &'static str, error: &DomainError) -> DirectSessionFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => DirectSessionFailureKind::Forbidden,
        DomainError::Validation { .. } => DirectSessionFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => DirectSessionFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => DirectSessionFailureKind::Internal,
    };
    DirectSessionFailure::new(operation, DirectSessionFailureStage::Validation, kind)
}

pub(crate) const fn repository(
    operation: &'static str,
    stage: DirectSessionFailureStage,
    error: &RepositoryError,
) -> DirectSessionFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => DirectSessionFailureKind::Conflict,
        RepositoryErrorKind::Constraint => DirectSessionFailureKind::InvalidRequest,
        RepositoryErrorKind::Forbidden => DirectSessionFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => DirectSessionFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => DirectSessionFailureKind::DependencyUnavailable,
        RepositoryErrorKind::CorruptData => DirectSessionFailureKind::Internal,
    };
    DirectSessionFailure::new(operation, stage, kind)
}

pub(crate) const fn matrix(operation: &'static str, error: MatrixFailure) -> DirectSessionFailure {
    let kind = match error.kind() {
        MatrixFailureKind::Conflict => DirectSessionFailureKind::Conflict,
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable => {
            DirectSessionFailureKind::DependencyUnavailable
        }
        MatrixFailureKind::UnknownCommit => DirectSessionFailureKind::UnknownCommit,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => DirectSessionFailureKind::Internal,
    };
    DirectSessionFailure::new(operation, DirectSessionFailureStage::Matrix, kind)
}

pub(crate) const fn failure(
    operation: &'static str,
    stage: DirectSessionFailureStage,
    kind: DirectSessionFailureKind,
) -> DirectSessionFailure {
    DirectSessionFailure::new(operation, stage, kind)
}
