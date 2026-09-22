//! 桌面端自己的本机日志：`<数据根>/logs/desktop.log`。Bridge 另写 `bridge.log`，两者同一目录，
//! 用户点「打开日志文件夹」就能整个发出来。只记状态、错误码和数量，不记消息内容和凭据。

use std::path::{Path, PathBuf};

use agent_room_bridge_local_adapter::{
    DEFAULT_LOG_FILE_CAP_BYTES, RotatingLogFile, bridge_log_root,
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt as _;
use tracing_subscriber::{
    EnvFilter, Layer as _, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

use crate::commands::DesktopCommandFailure;

const LOG_FILENAME: &str = "desktop.log";
const STDERR_FILTER: &str = "agent_room_desktop=warn";
const FILE_FILTER: &str = "agent_room_desktop=info";

/// 日志目录，桌面端与 Bridge 共用。
pub(crate) struct LogLocation {
    root: PathBuf,
}

impl LogLocation {
    pub(crate) fn new(data_root: &Path) -> Self {
        Self {
            root: bridge_log_root(data_root),
        }
    }

    pub(crate) fn file_path(&self) -> PathBuf {
        self.root.join(LOG_FILENAME)
    }

    /// 打开（创建）日志目录并安装订阅者。文件打不开时只写 stderr。
    pub(crate) fn install(&self) -> Option<PathBuf> {
        let stderr = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .with_filter(EnvFilter::new(STDERR_FILTER));
        let file = RotatingLogFile::open(self.file_path(), DEFAULT_LOG_FILE_CAP_BYTES).ok();
        let path = file.as_ref().map(RotatingLogFile::path);
        let file = file.map(|file| {
            tracing_subscriber::fmt::layer()
                .with_writer(move || file.clone())
                .with_ansi(false)
                .with_filter(EnvFilter::new(FILE_FILTER))
        });
        // 只装一次；测试或重复调用时保留已有的订阅者。
        let _ = tracing_subscriber::registry()
            .with(stderr)
            .with(file)
            .try_init();
        path
    }

    /// 用系统文件管理器打开日志目录。
    pub(crate) fn reveal(&self, app: &AppHandle) -> Result<(), DesktopCommandFailure> {
        std::fs::create_dir_all(&self.root)
            .map_err(|_| DesktopCommandFailure::new("desktop.logs.unavailable", true))?;
        app.opener()
            .open_path(self.root.to_string_lossy(), None::<&str>)
            .map_err(|_| DesktopCommandFailure::new("desktop.logs.open_failed", true))
    }
}

/// 打开日志文件夹，用户把里面的 `desktop.log` 与 `bridge.log` 发来即可排查。
#[tauri::command]
// Tauri 命令宏按值反序列化参数并提取运行时状态。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_open_logs(
    app: AppHandle,
    logs: tauri::State<'_, LogLocation>,
) -> Result<(), DesktopCommandFailure> {
    logs.reveal(&app)
}

#[cfg(test)]
mod tests {
    use super::LogLocation;

    #[test]
    fn 桌面日志与_bridge_日志同一目录() {
        let root = std::path::Path::new("data");
        let location = LogLocation::new(root);
        assert_eq!(location.file_path(), root.join("logs").join("desktop.log"));
    }
}
