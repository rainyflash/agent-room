mod agent_status;
mod config;
mod host_sessions;
mod ipc;
mod logging;
mod runtime;
mod runtime_files;
mod secure_storage;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    // Keep runtime failures observable without enabling verbose SDK/HTTP logs
    // that can contain credentials or message content.
    if let Some(path) = logging::install() {
        tracing::info!(path = %path.display(), "Bridge 启动，本机日志写入文件");
    }
    match runtime::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error_code = error.code(), %error, "Agent Room Bridge 启动失败");
            eprintln!("Agent Room Bridge 启动失败 [{}]：{error}", error.code());
            ExitCode::FAILURE
        }
    }
}
