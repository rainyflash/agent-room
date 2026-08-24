use std::{path::Path, time::Duration};

use sqlx::{
    SqlitePool,
    migrate::MigrateError,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use thiserror::Error;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: u32 = 4;

#[derive(Debug, Error)]
pub enum SqliteBridgeStorageOpenFailure {
    #[error("无法创建 Bridge 状态目录")]
    CreateDirectory(#[source] std::io::Error),
    #[error("无法打开 Bridge 状态数据库")]
    Connect(#[source] sqlx::Error),
    #[error("无法迁移 Bridge 状态数据库")]
    Migrate(#[source] MigrateError),
}

pub(crate) async fn open_pool(path: &Path) -> Result<SqlitePool, SqliteBridgeStorageOpenFailure> {
    let pool = connect_pool(path).await?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(SqliteBridgeStorageOpenFailure::Migrate)?;
    Ok(pool)
}

pub(crate) async fn open_handoff_pool(
    path: &Path,
) -> Result<SqlitePool, SqliteBridgeStorageOpenFailure> {
    let pool = connect_pool(path).await?;
    sqlx::migrate!("./handoff-migrations")
        .run(&pool)
        .await
        .map_err(SqliteBridgeStorageOpenFailure::Migrate)?;
    Ok(pool)
}

async fn connect_pool(path: &Path) -> Result<SqlitePool, SqliteBridgeStorageOpenFailure> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(SqliteBridgeStorageOpenFailure::CreateDirectory)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(BUSY_TIMEOUT);
    SqlitePoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .connect_with(options)
        .await
        .map_err(SqliteBridgeStorageOpenFailure::Connect)
}
