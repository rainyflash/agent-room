#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqliteFailureKind {
    NotFound,
    Conflict,
    Corrupt,
    Unavailable,
}

pub(crate) fn classify(error: &sqlx::Error) -> SqliteFailureKind {
    match error {
        sqlx::Error::RowNotFound => SqliteFailureKind::NotFound,
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::TypeNotFound { .. } => SqliteFailureKind::Corrupt,
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            SqliteFailureKind::Conflict
        }
        _ => SqliteFailureKind::Unavailable,
    }
}
