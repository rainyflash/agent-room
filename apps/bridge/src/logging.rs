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
/// 除了 Bridge 自己，还记下找回房间密钥两头的过程：各人物应请求重发了什么、没重发的原因
/// （`room_keys`），以及请别人重发、导入应答（`room_key_requests`）。否则“历史找回来没有”在日志里
/// 查不到。指令按前缀匹配目标，两个模块要分别写。每条都对应一次经 Olm 送达的请求或应答，不会刷屏。
const FILE_FILTER: &str = "agent_room_bridge=info,agent_room_matrix_adapter::room_keys=debug,\
     agent_room_matrix_adapter::room_key_requests=debug";
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
    use std::{
        io,
        sync::{Arc, Mutex, PoisonError},
    };

    use tracing_subscriber::EnvFilter;

    use super::{FILE_FILTER, file_filter, log_file_path};

    /// 把格式化好的日志收进内存，好看哪些条过了过滤规则。
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

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
        // EnvFilter 会重排指令，和同一串规则建出来的比。
        let default = EnvFilter::new(FILE_FILTER).to_string();
        for fallback in [None, Some("  ".to_owned()), Some("=[".to_owned())] {
            assert_eq!(file_filter(fallback).to_string(), default);
        }
    }

    #[test]
    fn 默认文件日志收下找回房间密钥两头的过程_别的模块照旧不记() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_env_filter(EnvFilter::new(FILE_FILTER))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "agent_room_matrix_adapter::room_keys", "应答者没有重发");
            tracing::info!(target: "agent_room_matrix_adapter::room_key_requests", "请求者请求重发");
            tracing::debug!(target: "agent_room_matrix_adapter::room_key_requests", "请求者没有请求");
            tracing::info!(target: "agent_room_bridge::runtime::isolated_messages", "Bridge 重读");
            tracing::debug!(target: "agent_room_bridge::runtime", "Bridge 的调试");
            tracing::info!(target: "agent_room_matrix_adapter::sdk", "别的模块");
        });
        let logged = String::from_utf8(
            captured
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("日志是 UTF-8");

        for kept in [
            "应答者没有重发",
            "请求者请求重发",
            "请求者没有请求",
            "Bridge 重读",
        ] {
            assert!(logged.contains(kept), "{kept} 应该记下：{logged}");
        }
        for dropped in ["Bridge 的调试", "别的模块"] {
            assert!(!logged.contains(dropped), "{dropped} 不该记下：{logged}");
        }
    }
}
