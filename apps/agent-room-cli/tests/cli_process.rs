use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    let directory = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_agent-room"))
        .env_remove("AGENT_ROOM_BRIDGE_VAULT_DIR")
        .env_remove("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE")
        .env("AGENT_ROOM_BRIDGE_DATA_DIR", directory.path())
        .env(
            "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE",
            format!("test.cli-{}", uuid::Uuid::now_v7()),
        )
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn 真实命令进程提供帮助和结构化参数错误() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("receive"));
    for args in [
        vec!["whoami", "--session", "invalid"],
        vec!["session", "open", "--name", "test", "--key", "invalid"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "bridge.ipc.session_id_invalid");
    }
}

#[test]
fn 发送必须明确声明用户授权或自动授权且失败有稳定退出码() {
    let output = run(&[
        "send",
        "--session",
        "test",
        "--room",
        "!test:room",
        "--text",
        "hello",
        "--submission-id",
        "test",
    ]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "cli.arguments_invalid");
    let output = run(&["doctor"]);
    assert_eq!(output.status.code(), Some(4));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(
        result["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("bridge.ipc.credentials_")
    );
}

#[test]
fn 新命令进程能读取加密凭据且配置缺失不回退钥匙串() {
    use agent_room_bridge_local_adapter::{
        IPC_INSTALLATION_ID_ACCOUNT, IPC_SHARED_SECRET_ACCOUNT, LocalSecretStore,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::io::Write as _;
    let root = tempfile::tempdir().unwrap();
    let mut key = tempfile::NamedTempFile::new_in(root.path()).unwrap();
    key.write_all(&[7; 32]).unwrap();
    let vault = root.path().join("vault");
    let service = format!("test.cli-{}", uuid::Uuid::now_v7());
    let store = LocalSecretStore::encrypted(&service, &vault, key.path()).unwrap();
    store
        .write(
            IPC_INSTALLATION_ID_ACCOUNT,
            &uuid::Uuid::now_v7().to_string(),
        )
        .unwrap();
    store
        .write(IPC_SHARED_SECRET_ACCOUNT, &URL_SAFE_NO_PAD.encode([8; 32]))
        .unwrap();
    for complete in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-room"));
        command
            .env("AGENT_ROOM_BRIDGE_DATA_DIR", root.path())
            .env("AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE", &service)
            .env("AGENT_ROOM_BRIDGE_VAULT_DIR", &vault)
            .env_remove("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE");
        if complete {
            command.env("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE", key.path());
        }
        let output = command.arg("doctor").output().unwrap();
        assert_eq!(output.status.code(), Some(4));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        let code = result["error"]["code"].as_str().unwrap();
        if complete {
            assert_eq!(code, "bridge.ipc.bridge_unavailable");
        } else {
            assert_eq!(code, "bridge.ipc.credentials_unavailable");
        }
    }
}
