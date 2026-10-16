//! 在隔离命名空间里启动真实 Bridge 进程，验证桌面端托管时它等桌面端交来电脑授权
//! （`specs/desktop-sign-in/design.md`）：不申请设备码，拿交来的授权注册这台电脑；
//! 桌面端要不到授权时写 `device-code`，Bridge 改走设备码。
//!
//! 每个用例使用独立的临时数据目录、文件保险库和安全存储服务名，不接触系统凭据库，
//! 也不会与本机已安装的 Agent Room 共用 IPC 端点。控制面是回环上的假服务。

use std::{
    io::{BufRead as _, BufReader, Read as _, Write as _},
    net::SocketAddr,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::json;
use uuid::Uuid;

const WAIT_TIMEOUT: Duration = Duration::from_mins(2);
/// 回环上没有服务监听的端口：身份服务和聊天服务器都连不上。
const CLOSED_LOOPBACK: &str = "http://127.0.0.1:9";
const GRANT: &str = "computer-grant";

/// 只认一份电脑授权的假控制面，记下收到的每一次注册。
fn spawn_control_plane(registrations: Arc<Mutex<Vec<String>>>) -> SocketAddr {
    let (address_sender, address) = mpsc::channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("测试运行时可创建");
        runtime.block_on(async move {
            let app = Router::new().route(
                "/auth/devices/register-with-grant",
                post(move |headers: HeaderMap| {
                    let registrations = registrations.clone();
                    async move {
                        let bearer = headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_owned();
                        registrations
                            .lock()
                            .expect("记录锁可用")
                            .push(bearer.clone());
                        if bearer != format!("Bearer {GRANT}") {
                            return (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({ "code": "device.invalid_authorization" })),
                            )
                                .into_response();
                        }
                        let now = now_unix_ms();
                        Json(json!({
                            "deviceId": "0198b601-77a1-7bb8-83eb-a8fe68c97e61",
                            "principalId": "0198b601-77a1-7bb8-83eb-a8fe68c97e62",
                            "matrixUserId": "@rainy:matrix.agent-room.localhost",
                            "displayName": "Rainy",
                            "avatarContentId": null,
                            "locale": "zh-CN",
                            "accessToken": "access-token",
                            "accessTokenExpiresAtUnixMs": now + 60 * 60 * 1_000,
                            "refreshToken": "refresh-token",
                            "refreshTokenExpiresAtUnixMs": now + 30 * 24 * 60 * 60 * 1_000
                        }))
                        .into_response()
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试监听器可绑定");
            address_sender
                .send(listener.local_addr().expect("测试地址存在"))
                .expect("测试线程在等地址");
            axum::serve(listener, app).await.expect("假控制面正常运行");
        });
    });
    address.recv().expect("假控制面已启动")
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
        key.write_all(&material).expect("保险库密钥可写入");
        Self {
            root,
            key,
            service: format!("test.supervisor-grant-{}", Uuid::now_v7()),
        }
    }

    fn vault(&self) -> PathBuf {
        self.root.path().join("vault")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn start_bridge(&self, control_plane: SocketAddr) -> Bridge进程 {
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
            .env("AGENT_ROOM_BRIDGE_DATA_DIR", self.data())
            .env("AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE", &self.service)
            .env("AGENT_ROOM_BRIDGE_VAULT_DIR", self.vault())
            .env("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE", self.key.path())
            .env("AGENT_ROOM_BRIDGE_SUPERVISED", "true")
            .env("AGENT_ROOM_BRIDGE_EXIT_WITH_SUPERVISOR", "true")
            .env("AGENT_ROOM_BRIDGE_DEVICE_AUTHORIZATION", "supervisor")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
    fn wait_for_output(&mut self, expected: &str) -> String {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            while let Ok(line) = self.received.try_recv() {
                let found = line.contains(expected);
                self.stdout.push(line.clone());
                if found {
                    return line;
                }
            }
            let exited = self.child.try_wait().expect("可查询 Bridge 进程状态");
            assert!(
                exited.is_none() && Instant::now() < deadline,
                "等待「{expected}」时 Bridge {}\n{}",
                exited.map_or_else(|| "超时".to_owned(), |status| format!("已退出：{status}")),
                self.output(),
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// 桌面端往 Bridge 的标准输入写一行，管道仍由桌面握着。
    fn write_line(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("标准输入是桌面持有的管道");
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .expect("可写入监督管道");
        stdin.flush().expect("可写入监督管道");
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("可查询 Bridge 进程状态") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn output(&self) -> String {
        format!(
            "标准输出：\n{}\n标准错误：\n{}",
            self.stdout.join("\n"),
            self.stderr.lock().expect("标准错误缓冲锁可用"),
        )
    }
}

impl Drop for Bridge进程 {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn 桌面端交来电脑授权就注册这台电脑_不申请设备码() {
    let registrations = Arc::new(Mutex::new(Vec::new()));
    let control_plane = spawn_control_plane(registrations.clone());
    let namespace = 隔离命名空间::new();
    let mut bridge = namespace.start_bridge(control_plane);
    bridge.wait_for_output(r#""event":"device_grant_required""#);

    bridge.write_line("device-grant used-grant");
    let failed = bridge.wait_for_output(r#""event":"device_grant_failed""#);
    assert!(
        failed.contains(r#""code":"bridge.identity_assertion_invalid""#),
        "{failed}"
    );

    bridge.write_line(&format!("device-grant {GRANT}"));
    bridge.wait_for_output(r#""event":"device_authorized""#);
    bridge.wait_for_output(r#""event":"ready""#);
    assert!(
        !bridge.output().contains("authorization_required"),
        "拿电脑授权注册时不出设备码\n{}",
        bridge.output()
    );
    assert_eq!(
        *registrations.lock().expect("记录锁可用"),
        ["Bearer used-grant", "Bearer computer-grant"]
    );

    bridge.write_line("exit");
    let status = bridge
        .wait_for_exit(Duration::from_secs(30))
        .unwrap_or_else(|| panic!("请它退出后 Bridge 应随之退出\n{}", bridge.output()));
    assert!(
        status.success(),
        "应当有序退出：{status}\n{}",
        bridge.output()
    );
}

#[test]
fn 桌面端要不到授权就改走设备码_授权途中桌面端退出也不再等() {
    let control_plane = spawn_control_plane(Arc::new(Mutex::new(Vec::new())));
    let namespace = 隔离命名空间::new();
    let mut bridge = namespace.start_bridge(control_plane);
    bridge.wait_for_output(r#""event":"device_grant_required""#);

    bridge.write_line("device-code");
    // 身份服务连不上：照旧报给桌面端、按退避重试。
    bridge.wait_for_output(r#""event":"server_unreachable""#);

    bridge.write_line("exit");
    let status = bridge
        .wait_for_exit(Duration::from_secs(30))
        .unwrap_or_else(|| panic!("授权途中请它退出，Bridge 应随之退出\n{}", bridge.output()));
    assert!(
        status.success(),
        "应当有序退出：{status}\n{}",
        bridge.output()
    );
}

fn now_unix_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时钟晚于 Unix epoch");
    i64::try_from(elapsed.as_millis()).expect("当前时间可用毫秒表示")
}
