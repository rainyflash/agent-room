use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match agent_room_control_plane::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("控制平面启动失败：{}", error.code());
            ExitCode::FAILURE
        }
    }
}
