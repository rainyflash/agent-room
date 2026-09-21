//! 在隔离命名空间里启动真实 Bridge 进程，验证设备会话刷新结果未知后能自行恢复。
//!
//! 每个用例使用独立的临时数据目录、文件保险库和安全存储服务名，不接触系统凭据库，
//! 也不会与本机已安装的 Agent Room 共用 IPC 端点。

use std::{
    io::{BufRead as _, BufReader, Read},
    net::SocketAddr,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_local_adapter::{DEVICE_SESSION_ACCOUNT, LocalSecretStore};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::{Uuid, Version};

const DEVICE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e60";
const WAIT_TIMEOUT: Duration = Duration::from_mins(2);
/// 回环上没有服务监听的端口：身份服务和 Matrix 在这些用例里都连不上。
const CLOSED_LOOPBACK: &str = "http://127.0.0.1:9";

#[derive(Clone, Copy)]
enum 刷新行为 {
    轮换,
    /// 首次请求已经轮换，回应却晚于客户端超时；同一幂等键的重试得到同一对令牌，换键视为重用。
    首次回应丢失,
    /// 服务端确认旧刷新令牌已不可用。
    拒绝,
}

#[derive(Default)]
struct 控制面记录 {
    requests: Vec<(Option<String>, Option<String>)>,
    committed_key: Option<String>,
}

struct 假控制面状态 {
    behavior: 刷新行为,
    record: Mutex<控制面记录>,
}

struct 假控制面 {
    address: SocketAddr,
    state: Arc<假控制面状态>,
    server: tokio::task::JoinHandle<()>,
}

impl 假控制面 {
    async fn start(behavior: 刷新行为) -> Self {
        let state = Arc::new(假控制面状态 {
            behavior,
            record: Mutex::new(控制面记录::default()),
        });
        let app = Router::new()
            .route("/auth/devices/refresh", post(refresh))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试控制面可监听回环端口");
        let address = listener.local_addr().expect("测试控制面地址可读取");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试控制面运行");
        });
        Self {
            address,
            state,
            server,
        }
    }

    fn requests(&self) -> Vec<(Option<String>, Option<String>)> {
        self.state
            .record
            .lock()
            .expect("控制面记录锁可用")
            .requests
            .clone()
    }
}

impl Drop for 假控制面 {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn refresh(State(state): State<Arc<假控制面状态>>, headers: HeaderMap) -> Response {
    let key = header(&headers, "idempotency-key");
    let first = {
        let mut record = state.record.lock().expect("控制面记录锁可用");
        record
            .requests
            .push((key.clone(), header(&headers, "authorization")));
        record.requests.len() == 1
    };
    match state.behavior {
        刷新行为::轮换 => rotated_credentials(),
        刷新行为::拒绝 => rejected(),
        刷新行为::首次回应丢失 if first => {
            state
                .record
                .lock()
                .expect("控制面记录锁可用")
                .committed_key
                .clone_from(&key);
            tokio::time::sleep(Duration::from_secs(5)).await;
            rotated_credentials()
        }
        刷新行为::首次回应丢失 => {
            let committed = state
                .record
                .lock()
                .expect("控制面记录锁可用")
                .committed_key
                .clone();
            if key.is_some() && key == committed {
                rotated_credentials()
            } else {
                rejected()
            }
        }
    }
}

fn rotated_credentials() -> Response {
    let now = now_unix_ms();
    Json(json!({
        "deviceId": DEVICE_ID,
        "principalId": "0198b601-77a1-7bb8-83eb-a8fe68c97e61",
        "matrixUserId": "@recovery:matrix.agent-room.localhost",
        "displayName": "恢复测试",
        "avatarContentId": null,
        "locale": "zh-CN",
        "accessToken": "new-access-token",
        "accessTokenExpiresAtUnixMs": now + 15 * 60 * 1_000,
        "refreshToken": "new-refresh-token",
        "refreshTokenExpiresAtUnixMs": now + 30 * 24 * 60 * 60 * 1_000
    }))
    .into_response()
}

fn rejected() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "code": "device.refresh_token_reuse" })),
    )
        .into_response()
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

struct 隔离命名空间 {
    root: tempfile::TempDir,
    key: tempfile::NamedTempFile,
    service: String,
}

impl 隔离命名空间 {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("临时目录可创建");
        let mut key = tempfile::NamedTempFile::new_in(root.path()).expect("保险库密钥可创建");
        let mut material = [0_u8; 32];
        getrandom::fill(&mut material).expect("系统随机源可用");
        std::io::Write::write_all(&mut key, &material).expect("保险库密钥可写入");
        Self {
            root,
            key,
            service: format!("test.refresh-recovery-{}", Uuid::now_v7()),
        }
    }

    fn vault(&self) -> PathBuf {
        self.root.path().join("vault")
    }

    fn store(&self) -> LocalSecretStore {
        LocalSecretStore::encrypted(&self.service, &self.vault(), self.key.path())
            .expect("隔离保险库可打开")
    }

    fn seed_device_session(&self, state: &str, access_token_expires_in_ms: i64) {
        let now = now_unix_ms();
        let persisted = json!({
            "version": 1,
            "state": state,
            "device_id": DEVICE_ID,
            "access_token": "old-access-token",
            "access_token_expires_at_unix_ms": now + access_token_expires_in_ms,
            "refresh_token": "old-refresh-token",
            "refresh_token_expires_at_unix_ms": now + 7 * 24 * 60 * 60 * 1_000
        });
        self.store()
            .write(DEVICE_SESSION_ACCOUNT, &persisted.to_string())
            .expect("设备会话可写入隔离保险库");
    }

    fn device_session(&self) -> Option<Value> {
        self.store()
            .read(DEVICE_SESSION_ACCOUNT)
            .expect("隔离保险库可读取")
            .map(|value| serde_json::from_str(&value).expect("设备会话是 JSON"))
    }

    fn start_bridge(&self, control_plane: SocketAddr, reset_device_session: bool) -> Bridge进程 {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-room-bridge"));
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("AGENT_ROOM_") {
                command.env_remove(name);
            }
        }
        command
            .env(
                "AGENT_ROOM_CONTROL_PLANE_URL",
                format!("http://{control_plane}/"),
            )
            .env("AGENT_ROOM_MATRIX_BASE_URL", format!("{CLOSED_LOOPBACK}/"))
            .env(
                "AGENT_ROOM_OIDC_ISSUER_URL",
                format!("{CLOSED_LOOPBACK}/realms/agent-room"),
            )
            .env("AGENT_ROOM_OIDC_DEVICE_CLIENT_ID", "agent-room-bridge")
            .env("AGENT_ROOM_BRIDGE_DATA_DIR", self.root.path().join("data"))
            .env("AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE", &self.service)
            .env("AGENT_ROOM_BRIDGE_VAULT_DIR", self.vault())
            .env("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE", self.key.path())
            .env("AGENT_ROOM_BRIDGE_SUPERVISED", "true")
            .env("AGENT_ROOM_BRIDGE_REQUEST_TIMEOUT_MS", "1500")
            .env("AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS", "200")
            .env("AGENT_ROOM_BRIDGE_RECONNECT_MAXIMUM_MS", "1000")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if reset_device_session {
            command.env("AGENT_ROOM_BRIDGE_RESET_DEVICE_SESSION", "true");
        }
        let mut child = command.spawn().expect("Bridge 进程可启动");
        let (lines, received) = mpsc::channel();
        let stdout = child.stdout.take().expect("Bridge 标准输出可读取");
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut stderr_pipe = child.stderr.take().expect("Bridge 标准错误可读取");
        let stderr_sink = stderr.clone();
        thread::spawn(move || {
            let mut captured = String::new();
            let _ = stderr_pipe.read_to_string(&mut captured);
            stderr_sink
                .lock()
                .expect("标准错误缓冲锁可用")
                .push_str(&captured);
        });
        Bridge进程 {
            child,
            received,
            stdout: Vec::new(),
            stderr,
        }
    }
}

struct Bridge进程 {
    child: Child,
    received: mpsc::Receiver<String>,
    stdout: Vec<String>,
    stderr: Arc<Mutex<String>>,
}

impl Bridge进程 {
    async fn wait_for_output(&mut self, expected: &str) {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            while let Ok(line) = self.received.try_recv() {
                let found = line.contains(expected);
                self.stdout.push(line);
                if found {
                    return;
                }
            }
            self.ensure_running(deadline, expected);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_until(&mut self, description: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        while !condition() {
            while let Ok(line) = self.received.try_recv() {
                self.stdout.push(line);
            }
            self.ensure_running(deadline, description);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn ensure_running(&mut self, deadline: Instant, waiting_for: &str) {
        let exited = self.child.try_wait().expect("可查询 Bridge 进程状态");
        if exited.is_some() || Instant::now() > deadline {
            thread::sleep(Duration::from_millis(200));
            panic!(
                "等待「{waiting_for}」时 Bridge {}\n标准输出：\n{}\n标准错误：\n{}",
                exited.map_or_else(|| "超时".to_owned(), |status| format!("已退出：{status}")),
                self.stdout.join("\n"),
                self.stderr.lock().expect("标准错误缓冲锁可用"),
            );
        }
    }
}

impl Drop for Bridge进程 {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 旧版本卡住的待决刷新在启动时自动对账并恢复就绪() {
    let namespace = 隔离命名空间::new();
    // Alpha 44 在结果未知时留下的就是这种没有尝试号的待决记录，旧令牌其实从未被服务端消费。
    namespace.seed_device_session("refresh_pending", 10 * 60 * 1_000);
    let control_plane = 假控制面::start(刷新行为::轮换).await;

    let mut bridge = namespace.start_bridge(control_plane.address, false);
    bridge.wait_for_output(r#""event":"ready""#).await;

    let requests = control_plane.requests();
    assert_eq!(requests.len(), 1, "启动时只应对账一次");
    let (key, authorization) = &requests[0];
    assert_eq!(authorization.as_deref(), Some("Bearer old-refresh-token"));
    assert_eq!(
        key.as_deref()
            .and_then(|key| Uuid::parse_str(key).ok())
            .and_then(|key| key.get_version()),
        Some(Version::SortRand),
        "对账请求必须带 UUID v7 幂等键"
    );
    let stored = namespace.device_session().expect("设备会话仍在");
    assert_eq!(stored["state"], "ready");
    assert_eq!(stored["refresh_token"], "new-refresh-token");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 刷新回应丢失后进程不退出并以同一尝试号重试到就绪() {
    let namespace = 隔离命名空间::new();
    // 访问令牌即将到期，启动即刷新；控制面已轮换但回应晚于客户端超时。
    namespace.seed_device_session("ready", 30 * 1_000);
    let control_plane = 假控制面::start(刷新行为::首次回应丢失).await;

    let mut bridge = namespace.start_bridge(control_plane.address, false);
    bridge
        .wait_until("同一尝试号的重试完成轮换", || {
            namespace
                .device_session()
                .is_some_and(|stored| stored["refresh_token"] == "new-refresh-token")
        })
        .await;

    let requests = control_plane.requests();
    assert_eq!(requests.len(), 2, "应只有首次请求和一次重试：{requests:?}");
    assert!(requests[0].0.is_some());
    assert_eq!(
        requests[0], requests[1],
        "重试必须沿用同一幂等键和旧刷新令牌"
    );
    assert_eq!(
        namespace.device_session().expect("设备会话仍在")["state"],
        "ready"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 服务端确认旧令牌不可用时清除凭据并进入设备授权() {
    let namespace = 隔离命名空间::new();
    namespace.seed_device_session("refresh_pending", 10 * 60 * 1_000);
    let control_plane = 假控制面::start(刷新行为::拒绝).await;

    let mut bridge = namespace.start_bridge(control_plane.address, false);
    // 身份服务在测试里连不上：Bridge 停在「首次设备授权」的重试上，而不是带着待决凭据退出。
    bridge
        .wait_for_output("bridge.identity_provider_unavailable")
        .await;

    assert_eq!(control_plane.requests().len(), 1);
    assert!(
        namespace.device_session().is_none(),
        "服务端确认旧令牌不可用后必须清除本机设备会话"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 桌面端要求重新授权时先清除本机凭据再进入设备授权() {
    let namespace = 隔离命名空间::new();
    namespace.seed_device_session("ready", 10 * 60 * 1_000);
    let control_plane = 假控制面::start(刷新行为::轮换).await;

    let mut bridge = namespace.start_bridge(control_plane.address, true);
    bridge
        .wait_for_output("bridge.identity_provider_unavailable")
        .await;

    assert!(namespace.device_session().is_none());
    assert!(
        control_plane.requests().is_empty(),
        "重新授权不应先用旧凭据刷新"
    );
}

fn now_unix_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时钟晚于 Unix epoch");
    i64::try_from(elapsed.as_millis()).expect("当前时间可用毫秒表示")
}
