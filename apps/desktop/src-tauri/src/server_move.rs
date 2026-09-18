//! 产品换到另一台服务器时，退役本机上属于旧服务器的状态。
//!
//! 本机的设备会话、默认 Agent 目标、CLI 资料和接收端配置都不记录来自哪台服务器；旧服务器上的
//! 身份在新服务器上并不存在，沿用它们会让 Bridge 带着旧 Agent 进大厅失败，引导一直停在“连接中”。
//! 桌面因此在数据目录里记下本机状态所属的控制面。启动时发现控制面变了，或者找到没有这份记录的
//! 旧版本状态，就在 Bridge 启动之前删除只对旧服务器有效的凭据，把其余状态整体移进
//! `retired/<时间>/`，Bridge 随后按第一次使用的流程重新授权。文件只移动、不删除。

use std::{
    fs::{self, DirBuilder, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt as _;

use agent_room_bridge_local_adapter::{LocalSecretStore, SERVER_BOUND_ACCOUNTS};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{desktop_config::DesktopBridgeConfig, human_session};

const RECORD_FILENAME: &str = "deployment.json";
const RETIRED_DIRECTORY: &str = "retired";
const RECORD_SCHEMA_VERSION: u8 = 1;
const BRIDGE_LOCK: &str = "bridge.lock";
/// 锁文件不带服务器状态，而且其他进程会继续用同一路径，所以留在原处。
const KEPT_ENTRIES: [&str; 4] = [
    RECORD_FILENAME,
    RETIRED_DIRECTORY,
    BRIDGE_LOCK,
    "matrix-store.lock",
];

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeploymentRecord {
    schema_version: u8,
    control_plane_url: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ServerMoveOutcome {
    /// 本机状态属于当前服务器。
    Unchanged,
    /// 第一次记录：没有需要退役的状态。
    Recorded,
    /// 旧服务器的状态已移到这个目录。
    Retired(PathBuf),
    /// 还有 Bridge 在用这些状态；本次启动不动它们，下次启动再处理。
    BridgeRunning,
}

/// 桌面启动时、Bridge 和登录恢复之前调用。失败不阻止启动：状态原样保留，下次启动再试。
pub(crate) fn retire_previous_server(config: &DesktopBridgeConfig) {
    let outcome = retire_previous_server_state(
        &config.data_root(),
        config.control_plane_url().as_str(),
        || forget_server_credentials(config),
    );
    match outcome {
        Ok(ServerMoveOutcome::Retired(folder)) => {
            eprintln!("previous server state retired to {}", folder.display());
        }
        Ok(ServerMoveOutcome::BridgeRunning) => {
            eprintln!("previous server state kept for now: a Bridge is still running");
        }
        Ok(ServerMoveOutcome::Unchanged | ServerMoveOutcome::Recorded) => {}
        Err(failure) => eprintln!(
            "previous server state retirement failed [{}]",
            failure.code()
        ),
    }
}

/// Bridge 的设备会话和默认 Agent 运行会话，以及桌面登录，都只对签发它们的服务器有效。
fn forget_server_credentials(config: &DesktopBridgeConfig) -> Result<(), ServerMoveFailure> {
    let unavailable = ServerMoveFailure::new("desktop.server_move.credentials_unavailable");
    let store = LocalSecretStore::from_environment(config.secure_storage_service().as_str());
    for account in SERVER_BOUND_ACCOUNTS {
        store.delete(account).map_err(|_| unavailable)?;
    }
    human_session::forget_stored_session(config).map_err(|_| unavailable)
}

/// 在 Bridge 启动前调用。`forget_credentials` 删除只对旧服务器有效的凭据，
/// 先于移动文件执行：它失败时什么都不动，下次启动整套重来。
pub(crate) fn retire_previous_server_state(
    data_root: &Path,
    control_plane_url: &str,
    forget_credentials: impl FnOnce() -> Result<(), ServerMoveFailure>,
) -> Result<ServerMoveOutcome, ServerMoveFailure> {
    let current = DeploymentRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        control_plane_url: control_plane_url.to_owned(),
    };
    let record_path = data_root.join(RECORD_FILENAME);
    if read_record(&record_path)?.as_ref() == Some(&current) {
        return Ok(ServerMoveOutcome::Unchanged);
    }
    if bridge_running(data_root)? {
        return Ok(ServerMoveOutcome::BridgeRunning);
    }
    forget_credentials()?;
    let entries = stale_entries(data_root)?;
    let outcome = if entries.is_empty() {
        ServerMoveOutcome::Recorded
    } else {
        let folder = data_root
            .join(RETIRED_DIRECTORY)
            .join(unix_seconds().to_string());
        create_private_directory(&folder)?;
        for entry in entries {
            let name = entry
                .file_name()
                .ok_or_else(|| ServerMoveFailure::new("desktop.server_move.path_invalid"))?;
            fs::rename(&entry, folder.join(name))
                .map_err(|_| ServerMoveFailure::new("desktop.server_move.retire_failed"))?;
        }
        ServerMoveOutcome::Retired(folder)
    };
    create_private_directory(data_root)?;
    write_record(&record_path, &current)?;
    Ok(outcome)
}

fn read_record(path: &Path) -> Result<Option<DeploymentRecord>, ServerMoveFailure> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(ServerMoveFailure::new(
                "desktop.server_move.record_unavailable",
            ));
        }
    };
    // 损坏或看不懂的记录不能证明状态属于当前服务器，按旧服务器的状态处理。
    Ok(serde_json::from_slice(&bytes).ok())
}

fn write_record(path: &Path, record: &DeploymentRecord) -> Result<(), ServerMoveFailure> {
    let parent = path
        .parent()
        .ok_or_else(|| ServerMoveFailure::new("desktop.server_move.path_invalid"))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|_| ServerMoveFailure::new("desktop.server_move.record_unavailable"))?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), record)
        .map_err(|_| ServerMoveFailure::new("desktop.server_move.record_unavailable"))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|_| ServerMoveFailure::new("desktop.server_move.record_unavailable"))?;
    temporary
        .persist(path)
        .map_err(|_| ServerMoveFailure::new("desktop.server_move.record_unavailable"))?;
    Ok(())
}

/// 默认 Bridge 和每个宿主 Agent 的 Bridge 都在自己的目录里持有独占锁。
fn bridge_running(data_root: &Path) -> Result<bool, ServerMoveFailure> {
    let mut locks = vec![data_root.join(BRIDGE_LOCK)];
    match fs::read_dir(data_root.join("host-agents")) {
        Ok(entries) => locks.extend(
            entries
                .flatten()
                .map(|entry| entry.path().join(BRIDGE_LOCK)),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(ServerMoveFailure::new(
                "desktop.server_move.lock_unavailable",
            ));
        }
    }
    for path in locks {
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(ServerMoveFailure::new(
                    "desktop.server_move.lock_unavailable",
                ));
            }
        };
        match file.try_lock() {
            Ok(()) => file
                .unlock()
                .map_err(|_| ServerMoveFailure::new("desktop.server_move.lock_unavailable"))?,
            Err(fs::TryLockError::WouldBlock) => return Ok(true),
            Err(fs::TryLockError::Error(_)) => {
                return Err(ServerMoveFailure::new(
                    "desktop.server_move.lock_unavailable",
                ));
            }
        }
    }
    Ok(false)
}

fn stale_entries(data_root: &Path) -> Result<Vec<PathBuf>, ServerMoveFailure> {
    let entries = match fs::read_dir(data_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(ServerMoveFailure::new("desktop.server_move.read_failed")),
    };
    let mut stale = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| ServerMoveFailure::new("desktop.server_move.read_failed"))?;
        let name = entry.file_name();
        if !KEPT_ENTRIES.iter().any(|kept| name.as_os_str() == *kept) {
            stale.push(entry.path());
        }
    }
    stale.sort();
    Ok(stale)
}

/// Bridge 拒绝 Unix 上权限宽于当前用户的数据目录，所以桌面先建目录时也只给自己。
fn create_private_directory(path: &Path) -> Result<(), ServerMoveFailure> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|_| ServerMoveFailure::new("desktop.server_move.directory_unavailable"))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServerMoveFailure {
    code: &'static str,
}

impl ServerMoveFailure {
    pub(crate) const fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use tempfile::tempdir;

    use super::{ServerMoveFailure, ServerMoveOutcome, retire_previous_server_state};

    const OLD: &str = "https://api.old.example/";
    const NEW: &str = "https://api.new.example/";

    fn seed_state(root: &std::path::Path) {
        fs::create_dir_all(root.join("desktop")).expect("目录可创建");
        fs::write(root.join("desktop").join("agent-target.json"), "{}").expect("目标可写");
        fs::create_dir_all(root.join("cli-profiles")).expect("目录可创建");
        fs::write(root.join("bridge.lock"), "pid=1").expect("锁文件可写");
    }

    #[test]
    fn 全新安装只记录服务器_不退役任何东西() {
        let root = tempdir().expect("临时目录有效");
        let data = root.path().join("Bridge");
        let forgotten = Cell::new(0);
        let outcome = retire_previous_server_state(&data, NEW, || {
            forgotten.set(forgotten.get() + 1);
            Ok(())
        })
        .expect("可记录");
        assert_eq!(outcome, ServerMoveOutcome::Recorded);
        assert!(data.join("deployment.json").is_file());
        let again = retire_previous_server_state(&data, NEW, || panic!("同一服务器不应清凭据"))
            .expect("可读取记录");
        assert_eq!(again, ServerMoveOutcome::Unchanged);
        assert_eq!(forgotten.get(), 1);
    }

    #[test]
    fn 没有记录的旧版本状态整体移走_锁文件留下() {
        let root = tempdir().expect("临时目录有效");
        seed_state(root.path());
        let outcome = retire_previous_server_state(root.path(), NEW, || Ok(())).expect("可退役");
        let ServerMoveOutcome::Retired(folder) = outcome else {
            panic!("应退役旧状态：{outcome:?}");
        };
        assert!(folder.join("desktop").join("agent-target.json").is_file());
        assert!(folder.join("cli-profiles").is_dir());
        assert!(!root.path().join("desktop").exists());
        assert!(root.path().join("bridge.lock").is_file());
        assert_eq!(
            retire_previous_server_state(root.path(), NEW, || panic!("已记录")).expect("可读取"),
            ServerMoveOutcome::Unchanged
        );
    }

    #[test]
    fn 服务器变了就退役_之前的退役目录保留() {
        let root = tempdir().expect("临时目录有效");
        retire_previous_server_state(root.path(), OLD, || Ok(())).expect("可记录");
        seed_state(root.path());
        let ServerMoveOutcome::Retired(first) =
            retire_previous_server_state(root.path(), NEW, || Ok(())).expect("可退役")
        else {
            panic!("换服务器应退役");
        };
        assert!(first.join("desktop").is_dir());
        assert!(root.path().join("retired").is_dir());
        assert!(!first.join("retired").exists());
    }

    #[test]
    fn 清凭据失败时什么都不动() {
        let root = tempdir().expect("临时目录有效");
        seed_state(root.path());
        let failure = retire_previous_server_state(root.path(), NEW, || {
            Err(ServerMoveFailure::new(
                "desktop.server_move.credentials_unavailable",
            ))
        })
        .expect_err("应报告失败");
        assert_eq!(
            failure.code(),
            "desktop.server_move.credentials_unavailable"
        );
        assert!(
            root.path()
                .join("desktop")
                .join("agent-target.json")
                .is_file()
        );
        assert!(!root.path().join("deployment.json").exists());
    }

    #[test]
    fn 有_bridge_持有锁时本次不动() {
        let root = tempdir().expect("临时目录有效");
        seed_state(root.path());
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.path().join("bridge.lock"))
            .expect("锁文件可打开");
        lock.lock().expect("可持有锁");
        assert_eq!(
            retire_previous_server_state(root.path(), NEW, || panic!("Bridge 运行时不应清凭据"))
                .expect("可检查"),
            ServerMoveOutcome::BridgeRunning
        );
        assert!(root.path().join("desktop").is_dir());
    }

    #[test]
    fn 看不懂的记录按旧服务器处理() {
        let root = tempdir().expect("临时目录有效");
        seed_state(root.path());
        fs::write(root.path().join("deployment.json"), "not json").expect("可写");
        assert!(matches!(
            retire_previous_server_state(root.path(), NEW, || Ok(())).expect("可退役"),
            ServerMoveOutcome::Retired(_)
        ));
    }
}
