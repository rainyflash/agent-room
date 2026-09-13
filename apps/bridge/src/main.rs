mod agent_status;
mod config;
mod host_sessions;
mod ipc;
mod runtime;
mod runtime_files;
mod secure_storage;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    // Keep runtime failures observable without enabling verbose SDK/HTTP logs
    // that can contain credentials or message content.
    tracing_subscriber::fmt()
        .with_env_filter("agent_room_bridge=warn")
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    match runtime::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Agent Room Bridge 启动失败 [{}]：{error}", error.code());
            ExitCode::FAILURE
        }
    }
}
