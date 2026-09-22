//! 在隔离命名空间里启动真实 Bridge 进程，验证它随监督它的桌面退出。
//!
//! 桌面把 Bridge 的标准输入接成自己持有的管道，桌面进程一退出（包括被强制结束）管道就关闭。
//! 每个用例使用独立的临时数据目录、文件保险库和安全存储服务名，不接触系统凭据库，
//! 也不会与本机已安装的 Agent Room 共用 IPC 端点。

use std::{
    fs::OpenOptions,
    io::{BufRead as _, BufReader, Read as _},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_local_adapter::{DEVICE_SESSION_ACCOUNT, LocalSecretStore};
use serde_json::json;
use uuid::Uuid;

const WAIT_TIMEOUT: Duration = Duration::from_mins(2);
/// 回环上没有服务监听的端口：用例里的 Bridge 不需要联系任何服务。
const CLOSED_LOOPBACK: &str = "http://127.0.0.1:9";

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
        let namespace = Self {
            root,
            key,
            service: format!("test.supervisor-exit-{}", Uuid::now_v7()),
        };
        namespace.seed_ready_device_session();
        namespace
    }

    fn vault(&self) -> PathBuf {
        self.root.path().join("vault")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    /// 已授权、令牌远未到期的设备会话：Bridge 启动时不用刷新，直接就绪。
    fn seed_ready_device_session(&self) {
        let now = now_unix_ms();
        let persisted = json!({
            "version": 1,
            "state": "ready",
            "device_id": "0198b601-77a1-7bb8-83eb-a8fe68c97e60",
            "access_token": "access-token",
            "access_token_expires_at_unix_ms": now + 60 * 60 * 1_000,
            "refresh_token": "refresh-token",
            "refresh_token_expires_at_unix_ms": now + 7 * 24 * 60 * 60 * 1_000
        });
        LocalSecretStore::encrypted(&self.service, &self.vault(), self.key.path())
            .expect("隔离保险库可打开")
            .write(DEVICE_SESSION_ACCOUNT, &persisted.to_string())
            .expect("设备会话可写入隔离保险库");
    }

    fn start_bridge(&self, exit_with_supervisor: bool) -> Bridge进程 {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-room-bridge"));
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("AGENT_ROOM_") {
                command.env_remove(name);
            }
        }
        command
            .env(
                "AGENT_ROOM_CONTROL_PLANE_URL",
                format!("{CLOSED_LOOPBACK}/"),
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
            .env(
                "AGENT_ROOM_BRIDGE_EXIT_WITH_SUPERVISOR",
                exit_with_supervisor.to_string(),
            )
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

    /// 实例锁是否已经空出来，也就是下一个 Bridge 能否启动。
    fn instance_lock_free(&self) -> bool {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.data().join("bridge.lock"))
            .expect("Bridge 创建过实例锁文件");
        let free = file.try_lock().is_ok();
        if free {
            file.unlock().expect("可释放实例锁");
        }
        free
    }
}

struct Bridge进程 {
    child: Child,
    received: mpsc::Receiver<String>,
    stdout: Vec<String>,
    stderr: Arc<Mutex<String>>,
}

impl Bridge进程 {
    fn wait_for_output(&mut self, expected: &str) {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            while let Ok(line) = self.received.try_recv() {
                let found = line.contains(expected);
                self.stdout.push(line);
                if found {
                    return;
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

    /// 监督它的桌面退出：桌面持有的管道一端随进程关闭。
    fn close_supervisor_pipe(&mut self) {
        drop(self.child.stdin.take().expect("标准输入是桌面持有的管道"));
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
fn 桌面退出后受监督的_bridge_有序退出并让出实例锁() {
    let namespace = 隔离命名空间::new();
    let mut bridge = namespace.start_bridge(true);
    bridge.wait_for_output(r#""event":"ready""#);
    assert!(
        !namespace.instance_lock_free(),
        "运行中的 Bridge 持有实例锁"
    );

    bridge.close_supervisor_pipe();

    let status = bridge
        .wait_for_exit(Duration::from_secs(30))
        .unwrap_or_else(|| panic!("桌面退出后 Bridge 应随之退出\n{}", bridge.output()));
    thread::sleep(Duration::from_millis(200));
    assert!(
        status.success(),
        "应当有序退出：{status}\n{}",
        bridge.output()
    );
    assert!(
        namespace.instance_lock_free(),
        "下一次启动的桌面要能立即起自己的 Bridge"
    );
}

#[test]
fn 未要求随桌面退出时标准输入关闭不影响_bridge() {
    let namespace = 隔离命名空间::new();
    let mut bridge = namespace.start_bridge(false);
    bridge.wait_for_output(r#""event":"ready""#);

    bridge.close_supervisor_pipe();

    assert!(
        bridge.wait_for_exit(Duration::from_secs(2)).is_none(),
        "手动运行或验收启动的 Bridge 不能因输入结束退出\n{}",
        bridge.output()
    );
}

fn now_unix_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时钟晚于 Unix epoch");
    i64::try_from(elapsed.as_millis()).expect("当前时间可用毫秒表示")
}
