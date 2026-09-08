use crate::{HostBinding, ReceptionFailure as CliFailure, ReceptionResult as CliResult};
use serde_json::Value;
use std::{ffi::OsString, path::Path};
use tokio::process::Command;

pub(crate) fn command(
    binding: &HostBinding,
    data_root: &Path,
    service: &str,
) -> CliResult<Command> {
    let mut command = Command::new(&binding.executable);
    let mcp = serde_json::to_string(&binding.mcp_executable)
        .map_err(|_| CliFailure::validation("receiver.executable_invalid"))?;
    let environment = mcp_environment(
        data_root,
        service,
        std::env::var_os("AGENT_ROOM_BRIDGE_VAULT_DIR"),
        std::env::var_os("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE"),
    )?;
    command
        .args(["exec", "--sandbox", "read-only", "--color", "never"])
        .arg("-c")
        .arg(format!("mcp_servers.agent_room.command={mcp}"))
        .args([
            "-c",
            "mcp_servers.agent_room.args=[]",
            "-c",
            "mcp_servers.agent_room.required=true",
            "-c",
            "mcp_servers.agent_room.tool_timeout_sec=150",
        ])
        .arg("-c")
        .arg(format!("mcp_servers.agent_room.env={environment}"))
        .args(["resume", &binding.task_id, "--json", "-"])
        .current_dir(&binding.workspace);
    Ok(command)
}
fn mcp_environment(
    data_root: &Path,
    service: &str,
    vault: Option<OsString>,
    key: Option<OsString>,
) -> CliResult<String> {
    let values = crate::host::mcp_environment(data_root, service, vault, key)?;
    let entries = values
        .into_iter()
        .map(|(name, value)| {
            serde_json::to_string(&value)
                .map(|value| format!("{name}={value}"))
                .map_err(|_| CliFailure::validation("receiver.vault_config_invalid"))
        })
        .collect::<CliResult<Vec<_>>>()?;
    Ok(format!("{{{}}}", entries.join(",")))
}

pub(crate) fn confirm_turn(stdout: &[u8], expected_task: &str) -> CliResult<()> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| CliFailure::local("receiver.host_output_invalid"))?;
    let mut bound = false;
    let mut completed = false;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let event: Value = serde_json::from_str(line)
            .map_err(|_| CliFailure::local("receiver.host_output_invalid"))?;
        match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                if event.get("thread_id").and_then(Value::as_str) != Some(expected_task) {
                    return Err(CliFailure::local("receiver.host_task_mismatch"));
                }
                bound = true;
            }
            Some("turn.completed") => completed = true,
            Some("turn.failed" | "error") => {
                return Err(CliFailure::local("receiver.host_turn_failed"));
            }
            Some(_) => {}
            None => return Err(CliFailure::local("receiver.host_output_invalid")),
        }
    }
    if bound && completed {
        Ok(())
    } else {
        Err(CliFailure::local("receiver.host_completion_missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 接收器为隔离的_mcp_子进程显式传递云端凭据位置() {
        let root = std::env::current_dir().unwrap();
        let value = mcp_environment(
            &root,
            "test.receiver",
            Some("vault with space".into()),
            Some("separate.key".into()),
        )
        .unwrap();
        assert!(value.contains("AGENT_ROOM_BRIDGE_VAULT_DIR=\"vault with space\""));
        assert!(value.contains("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE=\"separate.key\""));
        assert!(!value.contains("separate.key=\""));
        assert!(mcp_environment(&root, "test.receiver", Some("vault".into()), None).is_err());
    }
    #[test]
    fn 必须绑定指定任务且收到明确完成事件() {
        assert!(confirm_turn(b"{\"type\":\"thread.started\",\"thread_id\":\"task-a\"}\n{\"type\":\"turn.completed\"}\n", "task-a").is_ok());
        assert!(confirm_turn(b"{\"type\":\"thread.started\",\"thread_id\":\"task-b\"}\n{\"type\":\"turn.completed\"}\n", "task-a").is_err());
        assert!(confirm_turn(b"{\"type\":\"turn.completed\"}\n", "task-a").is_err());
        assert!(confirm_turn(b"{\"type\":\"thread.started\",\"thread_id\":\"task-a\"}\n{\"type\":\"turn.failed\"}\n", "task-a").is_err());
    }
    #[test]
    fn 不使用最近任务或取消权限边界且路径按单个参数传递() {
        let executable = std::env::current_exe().unwrap();
        let binding = HostBinding {
            host_type: agent_room_bridge_ipc::IpcReceptionHost::Codex,
            task_id: uuid::Uuid::now_v7().to_string(),
            executable: executable.clone(),
            mcp_executable: executable,
            workspace: std::env::current_dir().unwrap(),
        };
        let command = command(&binding, &binding.workspace, "test.receiver").unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--last" || arg.contains("dangerously"))
        );
        assert_eq!(
            &args[args.len() - 4..],
            ["resume", &binding.task_id, "--json", "-"]
        );
        assert!(args.contains(&"read-only".to_owned()));
    }
}
