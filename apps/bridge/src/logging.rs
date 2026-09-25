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
/// 排查时可以换掉文件日志的过滤规则（例如加上 matrix-sdk 的密钥分享）；不设就用默认的。
const FILE_FILTER_OVERRIDE: &str = "AGENT_ROOM_BRIDGE_LOG_FILTER";

/// 日志文件的位置：`<数据根>/logs/bridge.log`。
#[must_use]
pub(crate) fn log_file_path(data_root: &Path) -> PathBuf {
    bridge_log_root(data_root).join(LOG_FILENAME)
}

fn file_filter(configured: Option<String>) -> EnvFilter {
    configured
        .filter(|filter| !filter.trim().is_empty())
        .and_then(|filter| EnvFilter::try_new(filter).ok())
        .unwrap_or_else(|| EnvFilter::new(FILE_FILTER))
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
            .with_filter(file_filter(std::env::var(FILE_FILTER_OVERRIDE).ok()))
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(file)
        .init();
    path
}

#[cfg(test)]
mod tests {
    use super::{FILE_FILTER, file_filter, log_file_path};

    #[test]
    fn 日志文件在数据根的_logs_目录下() {
        let root = std::path::Path::new("root");
        assert_eq!(log_file_path(root), root.join("logs").join("bridge.log"));
    }

    #[test]
    fn 文件日志过滤规则可以换掉_空的或写错的用默认() {
        assert_eq!(
            file_filter(Some("agent_room_bridge=debug".to_owned())).to_string(),
            "agent_room_bridge=debug"
        );
        for fallback in [None, Some("  ".to_owned()), Some("=[".to_owned())] {
            assert_eq!(file_filter(fallback).to_string(), FILE_FILTER);
        }
    }
}
