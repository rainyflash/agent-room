use std::{path::Path, time::Duration};

use sqlx::{
    Sqlite, SqlitePool, Transaction,
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

/// 开启会写库的事务，并在开头就取得 WAL 写锁。
///
/// 默认 `BEGIN` 是延迟事务：先读后写时，SQLite 把读事务升级为写事务不会调用忙等待，
/// 另一连接正持有写锁时立即返回 `SQLITE_BUSY`，读取后另一连接已提交则返回
/// `SQLITE_BUSY_SNAPSHOT`。同一数据库文件上有多个连接池（投影与提交仓储）并发写，
/// 这种锁竞争会被误报为存储不可用。`BEGIN IMMEDIATE` 在未持有快照时取锁，
/// 最多按 [`BUSY_TIMEOUT`] 等待，超时才返回错误。
pub(crate) async fn begin_write(
    pool: &SqlitePool,
) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
    pool.begin_with("BEGIN IMMEDIATE").await
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
