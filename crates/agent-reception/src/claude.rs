use crate::{HostBinding, ReceptionFailure as Failure, ReceptionResult as Result};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::process::Command;

const ALLOWED: &str = "mcp__agent_room__agent_room_get_self,mcp__agent_room__agent_room_list_previews,mcp__agent_room__agent_room_wait_for_messages,mcp__agent_room__agent_room_send_message,mcp__agent_room__agent_room_publish_status";

pub(crate) async fn preflight(binding: &HostBinding) -> Result<()> {
    let mut command = Command::new(&binding.executable);
    command.arg("--help").current_dir(&binding.workspace);
    let output = tokio::time::timeout(Duration::from_secs(10), crate::host::execute(command, ""))
        .await
        .map_err(|_| Failure::local("receiver.host_probe_timeout"))??;
    validate_help(&output)
}
fn validate_help(output: &[u8]) -> Result<()> {
    let help =
        std::str::from_utf8(output).map_err(|_| Failure::local("receiver.host_output_invalid"))?;
    if [
        "--restricted",
        "--tools",
        "--strict-mcp-config",
        "--setting-sources",
        "dontAsk",
    ]
    .iter()
    .all(|flag| help.contains(flag))
    {
        Ok(())
    } else {
        Err(Failure::validation("receiver.claude_upgrade_required"))
    }
}
pub(crate) fn command(binding: &HostBinding, data_root: &Path, service: &str) -> Result<Command> {
    let environment = crate::host::mcp_environment(
        data_root,
        service,
        std::env::var_os("AGENT_ROOM_BRIDGE_VAULT_DIR"),
        std::env::var_os("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE"),
    )?;
    let config = json!({"mcpServers":{"agent_room":{"command":binding.mcp_executable,"args":[],"env":environment}}});
    let mut command = Command::new(&binding.executable);
    command
        .args([
            "--print",
            "--resume",
            &binding.task_id,
            "--output-format",
            "stream-json",
            "--verbose",
            "--restricted",
            "--tools",
            "",
            "--permission-mode",
            "dontAsk",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--settings",
            "{\"disableAllHooks\":true}",
            "--allowedTools",
            ALLOWED,
            "--mcp-config",
        ])
        .arg(config.to_string())
        .current_dir(&binding.workspace);
    Ok(command)
}
pub(crate) fn confirm_turn(stdout: &[u8], expected_task: &str) -> Result<()> {
    let text =
        std::str::from_utf8(stdout).map_err(|_| Failure::local("receiver.host_output_invalid"))?;
    let mut bound = false;
    let mut completed = false;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let event: Value = serde_json::from_str(line)
            .map_err(|_| Failure::local("receiver.host_output_invalid"))?;
        let session = event.get("session_id").and_then(Value::as_str);
        if session.is_some_and(|id| id != expected_task) {
            return Err(Failure::local("receiver.host_task_mismatch"));
        }
        match event.get("type").and_then(Value::as_str) {
            Some("system") if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                bound = session == Some(expected_task);
            }
            Some("result") => {
                if event.get("subtype").and_then(Value::as_str) != Some("success")
                    || event.get("is_error").and_then(Value::as_bool) != Some(false)
                    || session != Some(expected_task)
                    || event
                        .get("permission_denials")
                        .and_then(Value::as_array)
                        .is_some_and(|denials| !denials.is_empty())
                {
                    return Err(Failure::local("receiver.host_turn_failed"));
                }
                completed = true;
            }
            Some("error") => return Err(Failure::local("receiver.host_turn_failed")),
            Some(_) => {}
            None => return Err(Failure::local("receiver.host_output_invalid")),
        }
    }
    if bound && completed {
        Ok(())
    } else {
        Err(Failure::local("receiver.host_completion_missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 旧宿主缺少权限边界必须升级() {
        assert!(validate_help(b"--resume --print --mcp-config").is_err());
        assert!(
            validate_help(b"--restricted --tools --strict-mcp-config --setting-sources dontAsk")
                .is_ok()
        );
    }
    #[test]
    fn 完成事件必须属于同一任务且没有拒绝的权限() {
        let init = json!({"type":"system","subtype":"init","session_id":"task-a"});
        let mut result =
            json!({"type":"result","subtype":"success","is_error":false,"session_id":"task-a"});
        assert!(confirm_turn(format!("{init}\n{result}").as_bytes(), "task-a").is_ok());
        assert!(confirm_turn(result.to_string().as_bytes(), "task-a").is_err());
        assert!(confirm_turn(format!("{init}\n{result}").as_bytes(), "task-b").is_err());
        result["permission_denials"] =
            json!([{"tool_name":"mcp__agent_room__agent_room_send_message"}]);
        assert!(confirm_turn(format!("{init}\n{result}").as_bytes(), "task-a").is_err());
    }
    #[test]
    fn 只开放对话工具并固定宿主任务() {
        let binding = HostBinding {
            host_type: agent_room_bridge_ipc::IpcReceptionHost::ClaudeCode,
            task_id: uuid::Uuid::new_v4().to_string(),
            executable: std::env::current_exe().unwrap(),
            mcp_executable: std::env::current_exe().unwrap(),
            workspace: std::env::current_dir().unwrap(),
        };
        let command = command(&binding, &binding.workspace, "test.receiver").unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--resume", &binding.task_id])
        );
        assert!(args.windows(2).any(|pair| pair == ["--tools", ""]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--permission-mode", "dontAsk"])
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("bypass") || arg == "--continue")
        );
        let config: Value = serde_json::from_str(args.last().unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"]["agent_room"]["env"]["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"],
            "test.receiver"
        );
    }
}
