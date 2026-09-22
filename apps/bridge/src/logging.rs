//! Bridge 的日志去向：stderr 只留 warn 以上给监督它的桌面端看；本机日志文件从 info 起记，
//! 出了问题用户有东西可发。两处都不记消息内容和凭据，只记状态、错误码和数量。

use std::path::{Path, PathBuf};

use agent_room_bridge_local_adapter::{
    DEFAULT_LOG_FILE_CAP_BYTES, RotatingLogFile, bridge_log_root, resolve_bridge_data_root,
};
use tracing_subscriber::{
    EnvFilter, Layer as _, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

const LOG_FILENAME: &str = "bridge.log";
const STDERR_FILTER: &str = "agent_room_bridge=warn";
const FILE_FILTER: &str = "agent_room_bridge=info";

/// 日志文件的位置：`<数据根>/logs/bridge.log`。
#[must_use]
pub(crate) fn log_file_path(data_root: &Path) -> PathBuf {
    bridge_log_root(data_root).join(LOG_FILENAME)
}

/// 安装日志订阅者。文件打不开时只写 stderr，返回值说明文件日志是否可用。
pub(crate) fn install() -> Option<PathBuf> {
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_filter(EnvFilter::new(STDERR_FILTER));
    let file = resolve_bridge_data_root(|name| std::env::var(name).ok())
        .ok()
        .map(|root| log_file_path(&root))
        .and_then(|path| RotatingLogFile::open(path, DEFAULT_LOG_FILE_CAP_BYTES).ok());
    let path = file.as_ref().map(RotatingLogFile::path);
    let file = file.map(|file| {
        tracing_subscriber::fmt::layer()
            .with_writer(move || file.clone())
            .with_ansi(false)
            .with_filter(EnvFilter::new(FILE_FILTER))
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(file)
        .init();
    path
}

#[cfg(test)]
mod tests {
    use super::log_file_path;

    #[test]
    fn 日志文件在数据根的_logs_目录下() {
        let root = std::path::Path::new("root");
        assert_eq!(log_file_path(root), root.join("logs").join("bridge.log"));
    }
}
