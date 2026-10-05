//! 安装器换文件期间不启动桌面端。
//!
//! Windows 安装器（`windows/hooks.nsh`）停 Agent Room 之前，在安装目录里建一个标记文件并一直开着：
//! 不许别人打开，关掉就删，装完才放开。Agent 的 MCP 和命令行连不上 Bridge 时会把桌面端在后台
//! 拉起来；换文件的当口被拉起的桌面端会占住还没换的程序，还会和正在退出的 `WebView` 抢同一份
//! 本机数据。启动时发现标记被占着，就什么都不碰、直接退出。

use std::path::Path;

/// 与 `windows/hooks.nsh` 的 `AGENT_ROOM_INSTALLER_MARKER` 是同一个名字。
pub(crate) const INSTALLER_MARKER: &str = "installer-running.lock";

/// 别的进程正以不共享的方式开着这个文件。
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// 安装器正在换当前程序所在目录里的文件时返回 `true`。
pub(crate) fn installer_running_beside_current_executable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(installer_running))
        .unwrap_or(false)
}

/// 安装器正开着 `install_dir` 里的标记时返回 `true`。
///
/// 安装器意外退出不会留下标记（句柄关掉就删）。断电这类情况可能留下一个没人开着的，
/// 那时删掉它照常启动。别的打不开的情况也照常启动，免得桌面端被一个怪文件挡住再也起不来。
#[cfg(windows)]
pub(crate) fn installer_running(install_dir: &Path) -> bool {
    let marker = install_dir.join(INSTALLER_MARKER);
    match std::fs::File::open(&marker) {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&marker);
            false
        }
        Err(error) => error.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// 只有 Windows 安装器会在换文件时开着标记。
#[cfg(not(windows))]
pub(crate) fn installer_running(_install_dir: &Path) -> bool {
    false
}

/// 在桌面端日志里记一笔这次启动为什么直接退出，排查“桌面端没起来”时看得到。
pub(crate) fn log_skipped_start() {
    let Ok(config) = crate::desktop_config::DesktopBridgeConfig::from_environment() else {
        return;
    };
    crate::logging::LogLocation::new(&config.data_root()).install();
    tracing::info!("安装器正在换 Agent Room 的程序文件，这次启动直接退出");
}

#[cfg(test)]
mod tests {
    use super::{INSTALLER_MARKER, installer_running};

    #[test]
    fn 标记名和安装器钩子里的一致() {
        let hooks = include_str!("../windows/hooks.nsh");
        assert!(hooks.contains(&format!(
            "!define AGENT_ROOM_INSTALLER_MARKER \"{INSTALLER_MARKER}\""
        )));
    }

    #[test]
    fn 没有标记时照常启动() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!installer_running(directory.path()));
    }

    #[cfg(windows)]
    #[test]
    fn 安装器开着标记时不启动_放开后照常启动() {
        use std::os::windows::fs::OpenOptionsExt as _;

        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join(INSTALLER_MARKER);
        // 和钩子一样：写、不共享、关掉就删。
        let held = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&marker)
            .unwrap();

        assert!(installer_running(directory.path()));
        assert!(marker.exists());

        drop(held);
        assert!(!marker.exists());
        assert!(!installer_running(directory.path()));
    }

    #[cfg(windows)]
    #[test]
    fn 没人开着的旧标记删掉后照常启动() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join(INSTALLER_MARKER);
        std::fs::write(&marker, b"").unwrap();

        assert!(!installer_running(directory.path()));
        assert!(!marker.exists());
    }
}
