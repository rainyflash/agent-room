use sqlx::PgPool;

use crate::MigrationFailure;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../infra/migrations");

/// 应用全部待执行的只向前迁移。
///
/// # Errors
///
/// 数据库不可用、迁移校验和 SQL 执行失败时返回脱敏错误。
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrationFailure> {
    let mut connection = pool.acquire().await.map_err(MigrationFailure::Prepare)?;
    // PostgreSQL 默认搜索路径会在同名 Schema 出现后改变解析结果。迁移元数据必须固定在 public，
    // 否则第二次启动会误建 agent_room._sqlx_migrations 并重放首个迁移。
    sqlx::query("SELECT set_config('search_path', 'public', false)")
        .execute(&mut *connection)
        .await
        .map_err(MigrationFailure::Prepare)?;
    MIGRATOR
        .run(&mut *connection)
        .await
        .map_err(MigrationFailure::Apply)
}
