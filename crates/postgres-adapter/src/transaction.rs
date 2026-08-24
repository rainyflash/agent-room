use agent_room_application::persistence::{RepositoryError, RepositoryResult};
use sqlx::{Postgres, Transaction};

use crate::error::map_sqlx_error;

pub(crate) async fn finish<T>(
    transaction: Transaction<'_, Postgres>,
    result: RepositoryResult<T>,
    operation: &'static str,
) -> RepositoryResult<T> {
    match result {
        Ok(value) => {
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(value)
        }
        Err(error) => rollback(transaction, error, operation).await,
    }
}

async fn rollback<T>(
    transaction: Transaction<'_, Postgres>,
    error: RepositoryError,
    operation: &'static str,
) -> RepositoryResult<T> {
    transaction
        .rollback()
        .await
        .map_err(|rollback| map_sqlx_error(operation, &rollback))?;
    Err(error)
}
