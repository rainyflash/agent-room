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
    NotFound,
    Conflict,
    DependencyUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateRoomFailure {
    operation: &'static str,
    stage: PrivateRoomFailureStage,
    kind: PrivateRoomFailureKind,
}

impl PrivateRoomFailure {
    pub(crate) const fn new(
        operation: &'static str,
        stage: PrivateRoomFailureStage,
        kind: PrivateRoomFailureKind,
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

    pub const fn stage(self) -> PrivateRoomFailureStage {
        self.stage
    }

    pub const fn kind(self) -> PrivateRoomFailureKind {
        self.kind
    }
}

pub type PrivateRoomResult<T> = Result<T, PrivateRoomFailure>;

pub(crate) const fn domain(operation: &'static str, error: &DomainError) -> PrivateRoomFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => PrivateRoomFailureKind::Forbidden,
        DomainError::Validation { .. } => PrivateRoomFailureKind::InvalidRequest,
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
