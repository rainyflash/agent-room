//! 换账号时收走原账户留在这台电脑上的东西（`specs/sign-out/design.md` 第 4 步）。
//!
//! 托管的 Bridge 把这台电脑移出原来的账户、退出以后，删掉只对这个账户有效的凭据（设备会话、默认
//! Agent 的运行会话），再把数据目录里属于它的东西（人物、命令行和 MCP 的记录、消息、加密存储、附件、
//! 后台回复、默认 Agent 目标……）挪进 `retired/` 下的新目录，然后删掉那个目录。
//!
//! 开始之前先在数据目录里记一个标记，账户登录也清掉以后才去掉。中途退出了，下次启动时在 Bridge
//! 起来之前接着做完，连同还存着的账户登录一起清掉：不然新起的 Bridge 会拿原账户的登录要电脑授权，
//! 这台电脑又回到原账户。

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_local_adapter::{LocalSecretStore, SERVER_BOUND_ACCOUNTS};

use crate::{
    desktop_config::DesktopBridgeConfig,
    human_session,
    server_move::{self, BRIDGE_LOCK},
};

/// 换账号做到一半的标记。
const PENDING_MARKER: &str = "account-release.pending";
const RETIRED_DIRECTORY: &str = "retired";
/// 收走的东西先挪进 `retired/<时间><后缀>/`，再整个删掉；删不掉的留着，下次启动再删。
const RELEASED_SUFFIX: &str = "-account";
/// 不属于哪个账户、留在原处的：部署记录、换服务器时归档的目录、Bridge 的锁和本机连接的目录、
/// 桌面端日志（还开着）、托盘提示只出一次的记录，以及标记本身。
const KEPT_ENTRIES: [&str; 10] = [
    "deployment.json",
    RETIRED_DIRECTORY,
    BRIDGE_LOCK,
    "bridge.lock.pid",
    "matrix-store.lock",
    "matrix-store.lock.pid",
    "runtime",
    "logs",
    "tray-hint-shown",
    PENDING_MARKER,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AccountReleaseFailure {
    code: &'static str,
}

impl AccountReleaseFailure {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }
}

pub(crate) type AccountReleaseResult<T> = Result<T, AccountReleaseFailure>;

/// 开始换账号：记下标记。
pub(crate) fn begin(data_root: &Path) -> AccountReleaseResult<()> {
    server_move::create_private_directory(data_root)
        .map_err(|_| AccountReleaseFailure::new("desktop.release.marker_unavailable"))?;
    fs::write(data_root.join(PENDING_MARKER), b"1\n")
        .map_err(|_| AccountReleaseFailure::new("desktop.release.marker_unavailable"))
}

/// 换账号没开始就停下了（这台电脑没移出去）：去掉标记，什么都不收。
pub(crate) fn abandon(data_root: &Path) {
    remove_marker(data_root);
}

pub(crate) fn pending(data_root: &Path) -> bool {
    data_root.join(PENDING_MARKER).is_file()
}

/// 删掉只对原账户有效的凭据，再收走数据目录里属于它的东西。可以重复做：已经收走的不会再碰。
/// `forget_credentials` 先做，它失败时什么都不挪。
///
/// # Errors
///
/// 还有 Bridge 占着锁、凭据删不掉、或者有东西挪不走时返回错误；标记留着，下次再做。
pub(crate) fn retire_account_state(
    data_root: &Path,
    forget_credentials: impl FnOnce() -> AccountReleaseResult<()>,
) -> AccountReleaseResult<()> {
    if server_move::bridge_running(data_root)
        .map_err(|_| AccountReleaseFailure::new("desktop.release.lock_unavailable"))?
    {
        return Err(AccountReleaseFailure::new("desktop.release.bridge_running"));
    }
    forget_credentials()?;
    let entries = account_entries(data_root)?;
    if !entries.is_empty() {
        let folder = data_root
            .join(RETIRED_DIRECTORY)
            .join(format!("{}{RELEASED_SUFFIX}", unix_millis()));
        server_move::create_private_directory(&folder)
            .map_err(|_| AccountReleaseFailure::new("desktop.release.directory_unavailable"))?;
        let mut moved_all = true;
        for entry in entries {
            let Some(name) = entry.file_name() else {
                moved_all = false;
                continue;
            };
            if fs::rename(&entry, folder.join(name)).is_err() {
                tracing::warn!(entry = %entry.display(), "换账号时有东西挪不走");
                moved_all = false;
            }
        }
        if !moved_all {
            return Err(AccountReleaseFailure::new("desktop.release.retire_failed"));
        }
    }
    discard_released_folders(data_root);
    Ok(())
}

/// 换账号做完了：去掉标记。
pub(crate) fn finish(data_root: &Path) {
    remove_marker(data_root);
}

/// 桌面端启动时、Bridge 和登录恢复之前调用：上次换账号做到一半，就连同还存着的账户登录一起清掉、
/// 收走原账户的东西。顺带删掉以前收走、当时没删掉的目录。失败不挡启动，下次启动再试。
pub(crate) fn resume_interrupted(config: &DesktopBridgeConfig) {
    let data_root = config.data_root();
    if pending(&data_root) {
        let result = retire_account_state(&data_root, || {
            forget_account_credentials(config)?;
            human_session::forget_stored_session(config)
                .map_err(|_| AccountReleaseFailure::new("desktop.release.credentials_unavailable"))
        });
        match result {
            Ok(()) => {
                finish(&data_root);
                tracing::info!("上次换账号做到一半，已接着做完，需要重新登录");
            }
            Err(failure) => {
                tracing::warn!(
                    error_code = failure.code(),
                    "上次换账号做到一半，这次没做完"
                );
            }
        }
    } else {
        discard_released_folders(&data_root);
    }
}

/// Bridge 的设备会话和默认 Agent 的运行会话只对签发它们的账户有效。
pub(crate) fn forget_account_credentials(config: &DesktopBridgeConfig) -> AccountReleaseResult<()> {
    let store = LocalSecretStore::from_environment(config.secure_storage_service().as_str());
    for account in SERVER_BOUND_ACCOUNTS {
        store
            .delete(account)
            .map_err(|_| AccountReleaseFailure::new("desktop.release.credentials_unavailable"))?;
    }
    Ok(())
}

fn account_entries(data_root: &Path) -> AccountReleaseResult<Vec<PathBuf>> {
    let entries = match fs::read_dir(data_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(AccountReleaseFailure::new("desktop.release.read_failed")),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| AccountReleaseFailure::new("desktop.release.read_failed"))?;
        let name = entry.file_name();
        if !KEPT_ENTRIES.iter().any(|kept| name.as_os_str() == *kept) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

/// 删掉以前收走的原账户目录。还有别的进程开着里面的文件就先留着。
fn discard_released_folders(data_root: &Path) {
    let Ok(entries) = fs::read_dir(data_root.join(RETIRED_DIRECTORY)) else {
        return;
    };
    for entry in entries.flatten() {
        let released = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(RELEASED_SUFFIX));
        if released
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
            && let Err(error) = fs::remove_dir_all(entry.path())
        {
            tracing::info!(%error, folder = %entry.path().display(), "收走的原账户目录这次没删掉");
        }
    }
}

fn remove_marker(data_root: &Path) {
    match fs::remove_file(data_root.join(PENDING_MARKER)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(%error, "换账号的标记没删掉"),
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use tempfile::tempdir;

    use super::{AccountReleaseFailure, abandon, begin, finish, pending, retire_account_state};

    fn seed_account(root: &std::path::Path) {
        for directory in [
            "desktop",
            "host-agents",
            "cli-profiles",
            "mcp-joins",
            "messages",
            "matrix-store",
            "attachments",
            "handoffs",
            "receivers",
            "runtime",
            "logs",
            "retired",
        ] {
            fs::create_dir_all(root.join(directory)).expect("目录可创建");
        }
        fs::write(root.join("desktop").join("agent-target.json"), "{}").expect("目标可写");
        fs::write(root.join("owner.json"), "{}").expect("主人记录可写");
        fs::write(root.join("deployment.json"), "{}").expect("部署记录可写");
        fs::write(root.join("bridge.lock"), "pid=1").expect("锁文件可写");
        fs::write(root.join("logs").join("desktop.log"), "日志").expect("日志可写");
        fs::write(root.join("tray-hint-shown"), "1").expect("托盘提示可写");
        fs::create_dir_all(root.join("retired").join("1700000000")).expect("换服务器的归档可建");
    }

    #[test]
    fn 原账户的东西都收走_部署记录锁日志和换服务器的归档留下() {
        let root = tempdir().expect("临时目录有效");
        seed_account(root.path());
        begin(root.path()).expect("可记标记");
        let forgotten = Cell::new(0);

        retire_account_state(root.path(), || {
            forgotten.set(forgotten.get() + 1);
            Ok(())
        })
        .expect("可收走");

        assert_eq!(forgotten.get(), 1);
        for gone in [
            "desktop",
            "host-agents",
            "cli-profiles",
            "mcp-joins",
            "messages",
            "matrix-store",
            "attachments",
            "handoffs",
            "receivers",
            "owner.json",
        ] {
            assert!(!root.path().join(gone).exists(), "{gone} 应收走");
        }
        for kept in [
            "deployment.json",
            "bridge.lock",
            "runtime",
            "tray-hint-shown",
            "logs/desktop.log",
            "retired/1700000000",
        ] {
            assert!(root.path().join(kept).exists(), "{kept} 应留下");
        }
        let leftovers: Vec<_> = fs::read_dir(root.path().join("retired"))
            .expect("归档目录可读")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["1700000000"], "收走的原账户目录已经删掉");
        assert!(pending(root.path()), "账户登录清掉以前标记一直在");
        finish(root.path());
        assert!(!pending(root.path()));
    }

    #[test]
    fn 可以再做一次_已经收走的不再碰() {
        let root = tempdir().expect("临时目录有效");
        seed_account(root.path());
        retire_account_state(root.path(), || Ok(())).expect("第一次");
        fs::create_dir_all(root.path().join("mcp-joins")).expect("MCP 又写了一份");

        retire_account_state(root.path(), || Ok(())).expect("第二次");

        assert!(!root.path().join("mcp-joins").exists());
        assert!(root.path().join("deployment.json").is_file());
    }

    #[test]
    fn 凭据删不掉时什么都不挪() {
        let root = tempdir().expect("临时目录有效");
        seed_account(root.path());
        let failure = retire_account_state(root.path(), || {
            Err(AccountReleaseFailure::new(
                "desktop.release.credentials_unavailable",
            ))
        })
        .expect_err("应报告失败");
        assert_eq!(failure.code(), "desktop.release.credentials_unavailable");
        assert!(
            root.path()
                .join("desktop")
                .join("agent-target.json")
                .is_file()
        );
        assert!(root.path().join("owner.json").is_file());
    }

    #[test]
    fn 还有_bridge_占着锁时不动() {
        let root = tempdir().expect("临时目录有效");
        seed_account(root.path());
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.path().join("bridge.lock"))
            .expect("锁文件可打开");
        lock.lock().expect("可持有锁");

        let failure = retire_account_state(root.path(), || panic!("Bridge 还在时不删凭据"))
            .expect_err("应报告 Bridge 还在");

        assert_eq!(failure.code(), "desktop.release.bridge_running");
        assert!(root.path().join("owner.json").is_file());
    }

    #[test]
    fn 没开始就停下时去掉标记() {
        let root = tempdir().expect("临时目录有效");
        begin(root.path()).expect("可记标记");
        assert!(pending(root.path()));
        abandon(root.path());
        assert!(!pending(root.path()));
        abandon(root.path());
    }
}
