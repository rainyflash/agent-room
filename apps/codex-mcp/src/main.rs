use std::{process::ExitCode, sync::Arc};

use agent_room_bridge_local_adapter::{
    BridgeLocationFailure, bridge_data_root_from_environment, bridge_runtime_root,
};
use agent_room_codex_mcp::agent_room::{AgentRoomMcpServer, LocalBridgeToolClient};
use rmcp::{ServiceExt, transport::stdio};

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
    let data_root = bridge_data_root_from_environment().map_err(format_location_failure)?;
    let backend = Arc::new(LocalBridgeToolClient::system(bridge_runtime_root(
        &data_root,
    )));
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
