use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params_from_iter, types::Value};
use thiserror::Error;
use uuid::Uuid;

const STATE_DATABASE: &str = "matrix-sdk-state.sqlite3";
const RECOVERY_DIRECTORY: &str = "recovered-store-backups";
const QUERY_STATISTICS_TABLES: [&str; 2] = ["sqlite_stat1", "sqlite_stat4"];

pub(crate) fn recover_query_statistics(store_root: &Path) -> Result<bool, StoreRecoveryFailure> {
    let state_database = store_root.join(STATE_DATABASE);
    if !state_database.is_file() {
        return Ok(false);
    }

    let source = open_read_only(&state_database)?;
    let unreadable = unreadable_tables(&source)?;
    if unreadable.is_empty()
        || unreadable
            .iter()
            .any(|table| !QUERY_STATISTICS_TABLES.contains(&table.as_str()))
    {
        return Ok(false);
    }

    let recovery_id = Uuid::now_v7().to_string();
    let rebuilt_database = store_root.join(format!(".{STATE_DATABASE}.{recovery_id}.rebuilding"));
    rebuild_application_data(&source, &rebuilt_database)?;
    drop(source);

    let quarantine = store_root.join(RECOVERY_DIRECTORY).join(&recovery_id);
    fs::create_dir_all(&quarantine).map_err(StoreRecoveryFailure::Filesystem)?;
    let companions = database_companions(&state_database, &quarantine);
    let mut moved = Vec::new();
    for (original, backup) in companions.iter().filter(|(original, _)| original.exists()) {
        if let Err(error) = fs::rename(original, backup) {
            rollback_moves(&moved);
            let _ = fs::remove_file(&rebuilt_database);
            return Err(StoreRecoveryFailure::Filesystem(error));
        }
        moved.push((original.clone(), backup.clone()));
    }
    if let Err(error) = fs::rename(&rebuilt_database, &state_database) {
        rollback_moves(&moved);
        let _ = fs::remove_file(&rebuilt_database);
        return Err(StoreRecoveryFailure::Filesystem(error));
    }
    Ok(true)
}

fn open_read_only(path: &Path) -> Result<Connection, StoreRecoveryFailure> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(StoreRecoveryFailure::Database)
}

fn unreadable_tables(connection: &Connection) -> Result<BTreeSet<String>, StoreRecoveryFailure> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .map_err(StoreRecoveryFailure::Database)?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(StoreRecoveryFailure::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreRecoveryFailure::Database)?;
    let mut unreadable = BTreeSet::new();
    for name in names {
        let query = format!("SELECT COUNT(*) FROM {}", quote_identifier(&name));
        if connection
            .query_row(&query, [], |row| row.get::<_, i64>(0))
            .is_err()
        {
            unreadable.insert(name);
        }
    }
    Ok(unreadable)
}

fn rebuild_application_data(
    source: &Connection,
    destination: &Path,
) -> Result<(), StoreRecoveryFailure> {
    let schema = read_application_schema(source)?;
    let user_version = read_pragma_integer(source, "user_version")?;
    let application_id = read_pragma_integer(source, "application_id")?;
    let mut output = Connection::open_with_flags(
        destination,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(StoreRecoveryFailure::Database)?;
    let transaction = output
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StoreRecoveryFailure::Database)?;

    for object in schema.iter().filter(|object| object.kind == "table") {
        transaction
            .execute_batch(&object.sql)
            .map_err(StoreRecoveryFailure::Database)?;
        copy_table(source, &transaction, &object.name)?;
    }
    copy_sqlite_sequence(source, &transaction)?;
    for object in schema.iter().filter(|object| object.kind != "table") {
        transaction
            .execute_batch(&object.sql)
            .map_err(StoreRecoveryFailure::Database)?;
    }
    transaction
        .execute_batch(&format!(
            "PRAGMA user_version = {user_version}; PRAGMA application_id = {application_id};"
        ))
        .map_err(StoreRecoveryFailure::Database)?;
    transaction
        .commit()
        .map_err(StoreRecoveryFailure::Database)?;
    verify_integrity(&output)
}

fn read_application_schema(
    connection: &Connection,
) -> Result<Vec<SchemaObject>, StoreRecoveryFailure> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, sql
               FROM sqlite_schema
              WHERE sql IS NOT NULL
                AND name NOT LIKE 'sqlite_%'
              ORDER BY CASE type
                         WHEN 'table' THEN 0
                         WHEN 'index' THEN 1
                         WHEN 'trigger' THEN 2
                         WHEN 'view' THEN 3
                         ELSE 4
                       END,
                       name",
        )
        .map_err(StoreRecoveryFailure::Database)?;
    statement
        .query_map([], |row| {
            Ok(SchemaObject {
                kind: row.get(0)?,
                name: row.get(1)?,
                sql: row.get(2)?,
            })
        })
        .map_err(StoreRecoveryFailure::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreRecoveryFailure::Database)
}

fn copy_table(
    source: &Connection,
    destination: &rusqlite::Transaction<'_>,
    table: &str,
) -> Result<(), StoreRecoveryFailure> {
    let identifier = quote_identifier(table);
    let mut reader = source
        .prepare(&format!("SELECT * FROM {identifier}"))
        .map_err(StoreRecoveryFailure::Database)?;
    let column_count = reader.column_count();
    if column_count == 0 {
        return Ok(());
    }
    let placeholders = (1..=column_count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let insert = format!("INSERT INTO {identifier} VALUES ({placeholders})");
    let rows = reader
        .query_map([], |row| {
            (0..column_count)
                .map(|index| row.get::<_, Value>(index))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(StoreRecoveryFailure::Database)?;
    for row in rows {
        destination
            .execute(
                &insert,
                params_from_iter(row.map_err(StoreRecoveryFailure::Database)?),
            )
            .map_err(StoreRecoveryFailure::Database)?;
    }
    Ok(())
}

fn copy_sqlite_sequence(
    source: &Connection,
    destination: &rusqlite::Transaction<'_>,
) -> Result<(), StoreRecoveryFailure> {
    let exists = source
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'sqlite_sequence')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(StoreRecoveryFailure::Database)?;
    if !exists {
        return Ok(());
    }
    let mut statement = source
        .prepare("SELECT name, seq FROM sqlite_sequence")
        .map_err(StoreRecoveryFailure::Database)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(StoreRecoveryFailure::Database)?;
    for row in rows {
        let (name, sequence) = row.map_err(StoreRecoveryFailure::Database)?;
        destination
            .execute(
                "INSERT INTO sqlite_sequence(name, seq) VALUES (?1, ?2)",
                (name, sequence),
            )
            .map_err(StoreRecoveryFailure::Database)?;
    }
    Ok(())
}

fn read_pragma_integer(connection: &Connection, pragma: &str) -> Result<i64, StoreRecoveryFailure> {
    connection
        .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
        .map_err(StoreRecoveryFailure::Database)
}

fn verify_integrity(connection: &Connection) -> Result<(), StoreRecoveryFailure> {
    let mut statement = connection
        .prepare("PRAGMA integrity_check")
        .map_err(StoreRecoveryFailure::Database)?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(StoreRecoveryFailure::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreRecoveryFailure::Database)?;
    if messages == ["ok"] {
        Ok(())
    } else {
        Err(StoreRecoveryFailure::Integrity)
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn database_companions(database: &Path, quarantine: &Path) -> Vec<(PathBuf, PathBuf)> {
    let filename = database
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(STATE_DATABASE);
    ["", "-wal", "-shm"]
        .into_iter()
        .map(|suffix| {
            let name = format!("{filename}{suffix}");
            (database.with_file_name(&name), quarantine.join(name))
        })
        .collect()
}

fn rollback_moves(moved: &[(PathBuf, PathBuf)]) {
    for (original, backup) in moved.iter().rev() {
        let _ = fs::rename(backup, original);
    }
}

struct SchemaObject {
    kind: String,
    name: String,
    sql: String,
}

#[derive(Debug, Error)]
pub(crate) enum StoreRecoveryFailure {
    #[error("Matrix State Store 数据库操作失败")]
    Database(#[source] rusqlite::Error),
    #[error("Matrix State Store 文件替换失败")]
    Filesystem(#[source] std::io::Error),
    #[error("重建后的 Matrix State Store 未通过完整性检查")]
    Integrity,
}

#[cfg(test)]
mod tests {
    use std::{
        fs::OpenOptions,
        io::{Seek, SeekFrom, Write},
        path::Path,
    };

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{RECOVERY_DIRECTORY, STATE_DATABASE, recover_query_statistics};

    #[test]
    fn 仅查询统计页损坏时保留业务数据并隔离原文件() {
        let directory = tempdir().expect("临时目录可创建");
        let database = directory.path().join(STATE_DATABASE);
        create_analyzed_database(&database);
        corrupt_table_root_page(&database, "sqlite_stat4");

        assert!(recover_query_statistics(directory.path()).expect("统计页损坏可安全恢复"));

        let connection = Connection::open(&database).expect("恢复后的数据库可打开");
        let values = connection
            .prepare("SELECT value FROM records ORDER BY id")
            .expect("业务表可查询")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("业务行可读取")
            .collect::<Result<Vec<_>, _>>()
            .expect("业务行完整");
        assert_eq!(values, ["alpha", "beta", "gamma"]);
        let backups = std::fs::read_dir(directory.path().join(RECOVERY_DIRECTORY))
            .expect("损坏原件隔离目录存在")
            .collect::<Result<Vec<_>, _>>()
            .expect("隔离目录可读取");
        assert_eq!(backups.len(), 1);
    }

    #[test]
    fn 业务表损坏时拒绝自作主张重建() {
        let directory = tempdir().expect("临时目录可创建");
        let database = directory.path().join(STATE_DATABASE);
        create_analyzed_database(&database);
        corrupt_table_root_page(&database, "records");

        assert!(!recover_query_statistics(directory.path()).expect("业务损坏必须保持失败关闭"));
        assert!(!directory.path().join(RECOVERY_DIRECTORY).exists());
    }

    fn create_analyzed_database(path: &Path) {
        let connection = Connection::open(path).expect("数据库可创建");
        connection
            .execute_batch(
                "CREATE TABLE records(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
                 CREATE INDEX records_value_idx ON records(value);
                 INSERT INTO records(value) VALUES ('alpha'), ('beta'), ('gamma');
                 ANALYZE;",
            )
            .expect("测试数据库可建立并生成统计表");
        let stat4_exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'sqlite_stat4')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .expect("可检查 sqlite_stat4");
        assert!(stat4_exists, "测试 SQLite 必须启用 STAT4");
    }

    fn corrupt_table_root_page(path: &Path, table: &str) {
        let connection = Connection::open(path).expect("数据库可打开");
        let page_size = connection
            .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
            .expect("页大小可读取");
        let root_page = connection
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = ?1",
                [table],
                |row| row.get::<_, u64>(0),
            )
            .expect("目标根页存在");
        drop(connection);

        let offset = root_page.saturating_sub(1).saturating_mul(page_size);
        let mut file = OpenOptions::new()
            .write(true)
            .open(path)
            .expect("测试数据库可写");
        file.seek(SeekFrom::Start(offset)).expect("可定位目标页");
        file.write_all(&[0_u8; 32]).expect("可破坏目标页头");
        file.sync_all().expect("损坏页可落盘");
    }
}
