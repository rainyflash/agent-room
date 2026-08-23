use std::{env, process::ExitCode};

use agent_room_postgres_adapter::{MigrationFailure, run_migrations};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> ExitCode {
    let database_url = match env::var("AGENT_ROOM_MIGRATION_DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("缺少 AGENT_ROOM_MIGRATION_DATABASE_URL。");
            return ExitCode::FAILURE;
        }
    };

    match migrate(&database_url).await {
        Ok(()) => {
            println!("数据库迁移已处于最新版本。");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn migrate(database_url: &str) -> Result<(), MigrationFailure> {
    let pool = PgPoolOptions::new()
        .min_connections(0)
        .max_connections(1)
        .connect(database_url)
        .await
        .map_err(MigrationFailure::Connection)?;
    let result = run_migrations(&pool).await;
    pool.close().await;
    result
}
