mod config;
mod control_plane;
mod runtime;
mod secure_storage;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match runtime::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Agent Room Bridge 启动失败 [{}]：{error}", error.code());
            ExitCode::FAILURE
        }
    }
}
