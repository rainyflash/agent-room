use std::{net::SocketAddr, path::PathBuf, process::ExitCode, sync::Arc};

use agent_room_bridge_local_adapter::{
    BridgeLocationFailure, bridge_data_root_from_environment, bridge_runtime_root,
    secure_storage_service_from_environment,
};
use agent_room_mcp::agent_room::{AgentRoomMcpServer, LocalBridgeToolClient};
use agent_room_mcp::http::{HttpConfig, router};
use clap::Parser;
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(
    version,
    about = "Agent Room MCP: local stdio or private Streamable HTTP"
)]
struct Options {
    #[arg(long, requires_all = ["public_url", "authentication"])]
    http: Option<SocketAddr>,
    #[arg(long, requires = "http")]
    public_url: Option<String>,
    #[arg(
        long,
        requires = "http",
        group = "authentication",
        conflicts_with = "oauth_config"
    )]
    token_file: Option<PathBuf>,
    #[arg(long, requires = "http", group = "authentication")]
    oauth_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Agent Room MCP 启动失败：{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let options = Options::parse();
    let data_root = bridge_data_root_from_environment().map_err(format_location_failure)?;
    let secure_storage_service = secure_storage_service_from_environment()
        .map_err(|_| "Bridge 安全存储命名空间无效".to_owned())?;
    let backend = Arc::new(LocalBridgeToolClient::system_with_secure_storage_service(
        bridge_runtime_root(&data_root),
        secure_storage_service,
    ));
    if let Some(bind) = options.http {
        let public_url = options
            .public_url
            .as_deref()
            .ok_or("public URL is required")?;
        let config = if let Some(path) = options.oauth_config.as_deref() {
            HttpConfig::oauth(bind, public_url, path).await
        } else {
            HttpConfig::load(
                bind,
                public_url,
                options
                    .token_file
                    .as_deref()
                    .ok_or("authentication is required")?,
            )
        }
        .map_err(str::to_owned)?;
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .map_err(|error| error.to_string())?;
        eprintln!("Agent Room private MCP listening on {bind}");
        return axum::serve(listener, router(backend, config))
            .with_graceful_shutdown(async {
                if let Err(error) = tokio::signal::ctrl_c().await {
                    eprintln!("Shutdown signal unavailable: {error}");
                }
            })
            .await
            .map_err(|error| error.to_string());
    }
    let service = AgentRoomMcpServer::new(backend)
        .serve(stdio())
        .await
        .map_err(|error| format!("无法建立 STDIO 会话：{error}"))?;
    service
        .waiting()
        .await
        .map_err(|error| format!("STDIO 会话异常结束：{error}"))?;
    Ok(())
}

fn format_location_failure(failure: BridgeLocationFailure) -> String {
    format!(
        "Bridge 数据目录无效 [{}:{:?}]",
        failure.variable(),
        failure.kind()
    )
}
