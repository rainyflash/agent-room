use agent_room_bridge_core::ipc::{IpcInstallationId, IpcProtocolVersion};
use agent_room_bridge_ipc::{IpcFrame, IpcFrameCodec, IpcMethod, IpcResponse};
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
    /// 让下一次等待调用慢于客户端窗口，复现真实 Bridge 偶尔变慢的一次往返。
    slow_wait: Arc<AtomicBool>,
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
        let slow_wait = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(serve(
            listener,
            message_available.clone(),
            slow_wait.clone(),
            reads.clone(),
        ));
        Self {
            root,
            key,
            service,
            message_available,
            slow_wait,
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

async fn serve(
    listener: Listener,
    message_available: Arc<AtomicBool>,
    slow_wait: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
) {
    loop {
        let mut stream = listener.accept().await.unwrap();
        // 真实 Bridge 每条连接都单独成任务，立刻回到 accept。测试桩逐条串行会让第二个
        // 客户端反复撞上「管道忙」，凭空拉长本机往返，和被测行为无关。
        let message_available = message_available.clone();
        let slow_wait = slow_wait.clone();
        let reads = reads.clone();
        tokio::spawn(async move {
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
                    selected_version: IpcProtocolVersion::V4_0.into(),
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
                    selected_version: IpcProtocolVersion::V4_0.into(),
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
            let waiting = matches!(*method, IpcMethod::WaitInbox(_));
            let (IpcMethod::ReadInbox(request) | IpcMethod::WaitInbox(request)) = *method else {
                panic!("read must not acknowledge or send")
            };
            reads.fetch_add(1, Ordering::SeqCst);
            if waiting && slow_wait.swap(false, Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(2500)).await;
            }
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
        });
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
            attachment_name: None,
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
        "read {read:?} {read_line:?} exit {:?}; listen {listen:?} {listen_line:?} exit {:?}",
        reader.try_wait().unwrap(),
        listener.try_wait().unwrap()
    );
    assert!(read_line.is_empty() && listen_line.is_empty());
    assert_eq!(reader.try_wait().unwrap(), None, "read must keep waiting");
    assert_eq!(
        listener.try_wait().unwrap(),
        None,
        "listen must keep waiting"
    );
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
        .is_err(),
        "listen printed {listen_line:?} or exited {:?}",
        listener.try_wait().unwrap()
    );
    listener.kill().await.unwrap();
    listener.wait().await.unwrap();
    let count = harness.reads.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(harness.reads.load(Ordering::SeqCst), count);
}

/// 一次本机往返慢于 `--wait` 窗口时，窗口只是这一轮等待结束：监听不能输出任何一行，
/// 更不能因此退出。CI 上这条曾经表现为空闲期里突然多出一行 `agent.inbox.timeout`。
#[tokio::test]
async fn 单次等待慢于窗口时listen继续监听且不输出() {
    let harness = Harness::start();
    harness.slow_wait.store(true, Ordering::SeqCst);
    let mut listener = harness.command(&["listen", "--wait", "1"]);
    let mut listen_output = BufReader::new(listener.stdout.take().unwrap());
    let mut listen_line = String::new();
    assert!(
        timeout(
            Duration::from_secs(5),
            listen_output.read_line(&mut listen_line)
        )
        .await
        .is_err(),
        "listen printed {listen_line:?} or exited {:?}",
        listener.try_wait().unwrap()
    );
    assert_eq!(listener.try_wait().unwrap(), None, "一次慢往返不能终止监听");
    harness.message_available.store(true, Ordering::SeqCst);
    timeout(
        Duration::from_secs(5),
        listen_output.read_line(&mut listen_line),
    )
    .await
    .unwrap()
    .unwrap();
    let value: Value = serde_json::from_str(&listen_line).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["previews"][0]["eventId"], "$next");
    assert!(
        harness.reads.load(Ordering::SeqCst) > 2,
        "慢往返之后窗口照常继续轮询"
    );
    listener.kill().await.unwrap();
    listener.wait().await.unwrap();
}
