use agent_room_application::persistence::{RepositoryError, RepositoryErrorKind};
use agent_room_domain::DomainError;
use sqlx::error::DatabaseError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationFailure {
    #[error("无法连接迁移数据库")]
    Connection(#[source] sqlx::Error),
    #[error("无法准备迁移会话")]
    Prepare(#[source] sqlx::Error),
    #[error("无法应用数据库迁移")]
    Apply(#[source] sqlx::migrate::MigrateError),
}

pub(crate) fn map_sqlx_error(operation: &'static str, error: &sqlx::Error) -> RepositoryError {
    let kind = match error {
        sqlx::Error::RowNotFound => RepositoryErrorKind::NotFound,
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::TypeNotFound { .. } => RepositoryErrorKind::CorruptData,
        sqlx::Error::Database(database_error) => classify_database_error(database_error.as_ref()),
        _ => RepositoryErrorKind::Unavailable,
    };

    RepositoryError::new(operation, kind)
}

pub(crate) const fn map_domain_error(
    operation: &'static str,
    error: &DomainError,
) -> RepositoryError {
    let kind = match error {
        DomainError::Forbidden { .. } => RepositoryErrorKind::Forbidden,
        DomainError::InvariantViolation { .. }
        | DomainError::InvalidTransition { .. }
        | DomainError::Validation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => RepositoryErrorKind::Constraint,
    };
    RepositoryError::new(operation, kind)
}

fn classify_database_error(error: &dyn DatabaseError) -> RepositoryErrorKind {
    match error.code().as_deref() {
        Some("23505" | "40001" | "40P01") => RepositoryErrorKind::Conflict,
        Some("23502" | "23503" | "23514" | "23P01") => RepositoryErrorKind::Constraint,
        _ => RepositoryErrorKind::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::persistence::RepositoryErrorKind;

    use super::map_sqlx_error;

    #[test]
    fn 客户端错误不向应用层泄漏底层细节() {
        let timeout = map_sqlx_error("test", &sqlx::Error::PoolTimedOut);
        let missing = map_sqlx_error("test", &sqlx::Error::RowNotFound);

        assert_eq!(timeout.kind(), RepositoryErrorKind::Unavailable);
        assert_eq!(missing.kind(), RepositoryErrorKind::NotFound);
    }
}
