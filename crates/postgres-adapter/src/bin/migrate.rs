use std::{env, fs, process::ExitCode};

use agent_room_postgres_adapter::{MigrationFailure, run_migrations};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> ExitCode {
    let database_url = match read_database_url(&ProcessEnvironment) {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
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

trait EnvironmentSource {
    fn read(&self, name: &str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvironmentSource for ProcessEnvironment {
    fn read(&self, name: &str) -> Option<String> {
        env::var(name).ok()
    }
}

fn read_database_url(source: &impl EnvironmentSource) -> Result<String, &'static str> {
    let direct = source.read("AGENT_ROOM_MIGRATION_DATABASE_URL");
    let file = source
        .read("AGENT_ROOM_MIGRATION_DATABASE_URL_FILE")
        .filter(|value| !value.trim().is_empty());
    match (direct, file) {
        (Some(_), Some(_)) => Err("不得同时设置迁移数据库 URL 与对应的 _FILE 配置。"),
        (Some(value), None) => validate_database_url(value),
        (None, Some(path)) => {
            let value =
                fs::read_to_string(path).map_err(|_| "无法读取迁移数据库 URL Secret 文件。")?;
            validate_database_url(value.trim_end_matches(['\r', '\n']).to_owned())
        }
        (None, None) => Err("缺少 AGENT_ROOM_MIGRATION_DATABASE_URL。"),
    }
}

fn validate_database_url(value: String) -> Result<String, &'static str> {
    if value.trim().is_empty() || value.len() > 4_096 || value.contains(['\r', '\n', '\0']) {
        return Err("迁移数据库 URL 无效。");
    }
    Ok(value)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{EnvironmentSource, read_database_url};

    #[derive(Default)]
    struct MapEnvironment(BTreeMap<&'static str, String>);

    impl EnvironmentSource for MapEnvironment {
        fn read(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    #[test]
    fn 迁移_url_可以从文件加载() {
        let directory = tempfile::tempdir().expect("可创建临时目录");
        let path = directory.path().join("migration-url");
        std::fs::write(&path, "postgresql://migration:secret@postgres/agent_room\n")
            .expect("可写入 Secret");
        let environment = MapEnvironment(BTreeMap::from([(
            "AGENT_ROOM_MIGRATION_DATABASE_URL_FILE",
            path.to_string_lossy().into_owned(),
        )]));

        assert_eq!(
            read_database_url(&environment).expect("文件配置有效"),
            "postgresql://migration:secret@postgres/agent_room"
        );
    }

    #[test]
    fn 迁移_url_来源不得歧义() {
        let environment = MapEnvironment(BTreeMap::from([
            (
                "AGENT_ROOM_MIGRATION_DATABASE_URL",
                "postgresql://direct".to_owned(),
            ),
            (
                "AGENT_ROOM_MIGRATION_DATABASE_URL_FILE",
                "C:/run/secrets/migration-url".to_owned(),
            ),
        ]));

        assert!(read_database_url(&environment).is_err());
    }
}
