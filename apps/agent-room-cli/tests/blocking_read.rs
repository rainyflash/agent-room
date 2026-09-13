use agent_room_bridge_core::ipc::IpcInstallationId;
use agent_room_bridge_ipc::{IpcFrame, IpcFrameCodec, IpcMethod, IpcResponse, IpcVersion};
use agent_room_bridge_local_adapter::{
    IPC_INSTALLATION_ID_ACCOUNT, IPC_SHARED_SECRET_ACCOUNT, LocalIpcEndpoint, LocalSecretStore,
    bridge_runtime_root,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use interprocess::local_socket::{
    ListenerOptions,
    tokio::{Listener, prelude::*},
};
use serde_json::Value;
use std::{
    io::Write as _,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt as _, BufReader},
    process::{Child, Command},
    time::timeout,
};

const SESSION: &str = "01990d9e-8400-7000-8000-000000000010";

struct Harness {
    root: tempfile::TempDir,
    key: tempfile::NamedTempFile,
    service: String,
    message_available: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut key = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        key.write_all(&[7; 32]).unwrap();
        let service = format!("test.blocking-{}", uuid::Uuid::now_v7());
        let installation = IpcInstallationId::new(uuid::Uuid::now_v7().to_string()).unwrap();
        let store =
            LocalSecretStore::encrypted(&service, &root.path().join("vault"), key.path()).unwrap();
        store
            .write(IPC_INSTALLATION_ID_ACCOUNT, installation.as_str())
            .unwrap();
        store
            .write(IPC_SHARED_SECRET_ACCOUNT, &URL_SAFE_NO_PAD.encode([8; 32]))
            .unwrap();
        let runtime = bridge_runtime_root(root.path());
        std::fs::create_dir_all(&runtime).unwrap();
        let endpoint = LocalIpcEndpoint::from_installation(&runtime, &installation);
        let listener = ListenerOptions::new()
            .name(endpoint.to_name().unwrap())
            .create_tokio()
            .unwrap();
        let message_available = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(serve(listener, message_available.clone(), reads.clone()));
        Self {
            root,
            key,
            service,
            message_available,
            reads,
            server,
        }
    }

    fn command(&self, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_agent-room"))
            .env_remove("CODEX_THREAD_ID")
            .env("AGENT_ROOM_BRIDGE_DATA_DIR", self.root.path())
            .env("AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE", &self.service)
            .env(
                "AGENT_ROOM_BRIDGE_VAULT_DIR",
                self.root.path().join("vault"),
            )
            .env("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE", self.key.path())
            .args(args)
            .args(["--session", SESSION, "--after", "$last"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn serve(listener: Listener, message_available: Arc<AtomicBool>, reads: Arc<AtomicUsize>) {
    loop {
        let mut stream = listener.accept().await.unwrap();
        let IpcFrame::ClientHello {
            requested_scopes, ..
        } = IpcFrameCodec::read(&mut stream).await.unwrap()
        else {
            panic!("client hello")
        };
        IpcFrameCodec::write(
            &mut stream,
            &IpcFrame::ServerChallenge {
                challenge_id: SESSION.parse().unwrap(),
                challenge: URL_SAFE_NO_PAD.encode([9; 32]),
                selected_version: IpcVersion { major: 3, minor: 0 },
                granted_scopes: requested_scopes.clone(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            IpcFrameCodec::read(&mut stream).await.unwrap(),
            IpcFrame::ClientProof { .. }
        ));
        IpcFrameCodec::write(
            &mut stream,
            &IpcFrame::ServerReady {
                server_instance_id: SESSION.parse().unwrap(),
                selected_version: IpcVersion { major: 3, minor: 0 },
                granted_scopes: requested_scopes,
            },
        )
        .await
        .unwrap();
        let IpcFrame::Request {
            correlation_id,
            method,
        } = IpcFrameCodec::read(&mut stream).await.unwrap()
        else {
            panic!("request")
        };
        let IpcMethod::WithSession { session_id, method } = method else {
            panic!("session required")
        };
        assert_eq!(session_id, SESSION);
        let IpcMethod::ReadInbox(request) = *method else {
            panic!("read must not acknowledge or send")
        };
        reads.fetch_add(1, Ordering::SeqCst);
        let previews = if message_available.load(Ordering::SeqCst)
            && request.after_event_id.as_deref() == Some("$last")
        {
            vec![message()]
        } else {
            vec![]
        };
        IpcFrameCodec::write(
            &mut stream,
            &IpcFrame::Response {
                correlation_id,
                result: IpcResponse::MessagePreviews {
                    previews,
                    next_cursor: None,
                },
            },
        )
        .await
        .unwrap();
    }
}

fn message() -> agent_room_bridge_ipc::IpcMessagePreviewSummary {
    use agent_room_bridge_ipc::{
        IpcActorSummary, IpcContentReference, IpcConversationMessage, IpcMessagePreviewSummary,
        IpcMessageSensitivity,
    };
    IpcMessagePreviewSummary {
        event_id: "$next".into(),
        message_id: SESSION.into(),
        room_id: "!room:example.test".into(),
        actor: IpcActorSummary::Human {
            principal_id: SESSION.into(),
            display_name: "Owner".into(),
            matrix_user_id: "@owner:example.test".into(),
            avatar_url: None,
        },
        conversation: Some(IpcConversationMessage {
            text: "hello".into(),
            mentions: vec![],
        }),
        reply_to_message_id: None,
        created_at_unix_ms: 1,
        title: "hello".into(),
        summary: "hello".into(),
        content: IpcContentReference {
            content_id: SESSION.into(),
            digest_sha256: "0".repeat(64),
            media_type: "text/plain".into(),
            size_bytes: 5,
        },
        language: None,
        sensitivity: IpcMessageSensitivity::Normal,
        risk_flags: vec![],
    }
}

#[tokio::test]
async fn 真实cli进程空闲三十秒零输出收到消息才返回且listen不会输出空批次() {
    let harness = Harness::start();
    let mut reader = harness.command(&["read"]);
    let mut listener = harness.command(&["listen", "--wait", "1"]);
    let mut read_output = BufReader::new(reader.stdout.take().unwrap());
    let mut listen_output = BufReader::new(listener.stdout.take().unwrap());
    let mut read_line = String::new();
    let mut listen_line = String::new();
    let (read, listen) = tokio::join!(
        timeout(
            Duration::from_secs(30),
            read_output.read_line(&mut read_line)
        ),
        timeout(
            Duration::from_secs(30),
            listen_output.read_line(&mut listen_line)
        ),
    );
    assert!(
        read.is_err() && listen.is_err(),
        "unexpected output: {read_line} / {listen_line}"
    );
    assert!(read_line.is_empty() && listen_line.is_empty());
    assert!(reader.try_wait().unwrap().is_none());
    assert!(listener.try_wait().unwrap().is_none());
    assert!(harness.reads.load(Ordering::SeqCst) > 25);
    harness.message_available.store(true, Ordering::SeqCst);
    timeout(
        Duration::from_secs(5),
        read_output.read_line(&mut read_line),
    )
    .await
    .unwrap()
    .unwrap();
    timeout(
        Duration::from_secs(5),
        listen_output.read_line(&mut listen_line),
    )
    .await
    .unwrap()
    .unwrap();
    for line in [&read_line, &listen_line] {
        let value: Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["previews"][0]["eventId"], "$next");
    }
    assert!(
        timeout(Duration::from_secs(5), reader.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    listen_line.clear();
    assert!(
        timeout(
            Duration::from_secs(2),
            listen_output.read_line(&mut listen_line)
        )
        .await
        .is_err()
    );
    listener.kill().await.unwrap();
    listener.wait().await.unwrap();
    let count = harness.reads.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(harness.reads.load(Ordering::SeqCst), count);
}
