use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use agent_room_bridge_ipc::{IpcBridgeState, IpcErrorCategory, IpcMethod, IpcResponse};
use tokio::time::Instant;

use super::{DesktopLauncher, DesktopLaunchingClient, READY_TIMEOUT, RELAUNCH_INTERVAL};
use crate::bridge::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture};

fn failure(code: &str) -> BridgeToolFailure {
    BridgeToolFailure::new(
        code,
        IpcErrorCategory::DependencyUnavailable,
        true,
        BTreeMap::new(),
    )
}

/// 假 Bridge：没在运行时一律连不上；在运行时按 `state` 应答。
#[derive(Clone)]
struct FakeBridge {
    running: Arc<AtomicBool>,
    state: Arc<Mutex<IpcBridgeState>>,
    refusal: Option<&'static str>,
}

impl FakeBridge {
    fn absent() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(IpcBridgeState::Ready)),
            refusal: None,
        }
    }

    fn running() -> Self {
        let bridge = Self::absent();
        bridge.running.store(true, Ordering::SeqCst);
        bridge
    }
}

impl BridgeToolClient for FakeBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        Box::pin(async move {
            if let Some(code) = self.refusal {
                return Err(failure(code));
            }
            if !self.running.load(Ordering::SeqCst) {
                return Err(failure("bridge.ipc.bridge_unavailable"));
            }
            let state = *self.state.lock().expect("状态锁可用");
            if method != IpcMethod::BridgeStatus && state != IpcBridgeState::Ready {
                return Err(failure("bridge.agent_runtime_unavailable"));
            }
            Ok(IpcResponse::BridgeStatus {
                state,
                started_at_unix_ms: 1,
                failure_code: None,
            })
        })
    }
}

/// 假桌面端：启动后过 `startup` 这么久，Bridge 以 `state` 上线。
struct FakeDesktop {
    bridge: FakeBridge,
    startup: Duration,
    state: IpcBridgeState,
    broken: bool,
    launches: Arc<AtomicUsize>,
}

impl FakeDesktop {
    fn starting_in(bridge: &FakeBridge, startup: Duration) -> Self {
        Self {
            bridge: bridge.clone(),
            startup,
            state: IpcBridgeState::Ready,
            broken: false,
            launches: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl DesktopLauncher for FakeDesktop {
    fn launch(&self) -> io::Result<()> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        if self.broken {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let bridge = self.bridge.clone();
        let (startup, state) = (self.startup, self.state);
        tokio::spawn(async move {
            tokio::time::sleep(startup).await;
            *bridge.state.lock().expect("状态锁可用") = state;
            bridge.running.store(true, Ordering::SeqCst);
        });
        Ok(())
    }
}

fn client(
    bridge: &FakeBridge,
    desktop: FakeDesktop,
) -> (
    DesktopLaunchingClient<FakeBridge, FakeDesktop>,
    Arc<AtomicUsize>,
) {
    let launches = Arc::clone(&desktop.launches);
    (
        DesktopLaunchingClient::new(bridge.clone(), desktop),
        launches,
    )
}

#[tokio::test(start_paused = true)]
async fn bridge_在的时候不拉起桌面端() {
    let bridge = FakeBridge::running();
    let (client, launches) = client(&bridge, FakeDesktop::starting_in(&bridge, Duration::ZERO));

    client.invoke(IpcMethod::GetSelf).await.expect("直接成功");

    assert_eq!(launches.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn bridge_不在时拉起桌面端_就绪后重做这次调用() {
    let bridge = FakeBridge::absent();
    let (client, launches) = client(
        &bridge,
        FakeDesktop::starting_in(&bridge, Duration::from_secs(3)),
    );

    client.invoke(IpcMethod::GetSelf).await.expect("拉起后成功");

    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn 几个调用同时发现_bridge_不在_只拉起一次() {
    let bridge = FakeBridge::absent();
    let (client, launches) = client(
        &bridge,
        FakeDesktop::starting_in(&bridge, Duration::from_secs(3)),
    );

    let (first, second) = tokio::join!(
        client.invoke(IpcMethod::GetSelf),
        client.invoke(IpcMethod::BridgeStatus)
    );

    first.expect("第一个调用成功");
    second.expect("第二个调用也等到了");
    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn 一分钟还没就绪就放弃_两分钟内不再拉起() {
    let bridge = FakeBridge::absent();
    let (client, launches) = client(
        &bridge,
        FakeDesktop::starting_in(&bridge, Duration::from_hours(1)),
    );

    let started = Instant::now();
    let first = client.invoke(IpcMethod::GetSelf).await.expect_err("没起来");
    assert_eq!(first.code(), "bridge.ipc.bridge_unavailable");
    assert!(started.elapsed() >= READY_TIMEOUT);

    // 窗口已过、还没到重试间隔：马上报错，不再等，也不再拉起。
    let again = Instant::now();
    client
        .invoke(IpcMethod::GetSelf)
        .await
        .expect_err("还是没起来");
    assert!(again.elapsed() < Duration::from_secs(1));
    assert_eq!(launches.load(Ordering::SeqCst), 1);

    tokio::time::sleep(RELAUNCH_INTERVAL).await;
    client
        .invoke(IpcMethod::GetSelf)
        .await
        .expect_err("第二次也没起来");
    assert_eq!(launches.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn 桌面端启动不了就照原样报连不上() {
    let bridge = FakeBridge::absent();
    let mut desktop = FakeDesktop::starting_in(&bridge, Duration::ZERO);
    desktop.broken = true;
    let (client, launches) = client(&bridge, desktop);

    let started = Instant::now();
    let failure = client
        .invoke(IpcMethod::GetSelf)
        .await
        .expect_err("启动不了");

    assert_eq!(failure.code(), "bridge.ipc.bridge_unavailable");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn 别的失败不拉起桌面端() {
    let mut bridge = FakeBridge::running();
    bridge.refusal = Some("bridge.ipc.authentication_rejected");
    let (client, launches) = client(&bridge, FakeDesktop::starting_in(&bridge, Duration::ZERO));

    let failure = client.invoke(IpcMethod::GetSelf).await.expect_err("被拒绝");

    assert_eq!(failure.code(), "bridge.ipc.authentication_rejected");
    assert_eq!(launches.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn 拉起后_bridge_报离线就不再干等() {
    let bridge = FakeBridge::absent();
    let mut desktop = FakeDesktop::starting_in(&bridge, Duration::from_secs(2));
    desktop.state = IpcBridgeState::Offline;
    let (client, launches) = client(&bridge, desktop);

    let started = Instant::now();
    let failure = client.invoke(IpcMethod::GetSelf).await.expect_err("离线");

    assert_eq!(failure.code(), "bridge.agent_runtime_unavailable");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
}

/// 用完即删的临时目录。
struct ScratchDirectory(std::path::PathBuf);

impl ScratchDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-room-desktop-launch-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&path).expect("可建临时目录");
        Self(path)
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
#[test]
fn 找的是装在同一目录的桌面端_没有就不拉起() {
    let directory = ScratchDirectory::new();
    let mcp = directory.0.join("agent-room-mcp.exe");
    assert!(super::InstalledDesktop::beside(&mcp).is_none());

    std::fs::write(directory.0.join("agent-room-desktop.exe"), b"").expect("可写");
    let desktop = super::InstalledDesktop::beside(&mcp).expect("找到桌面端");
    assert_eq!(desktop.target, directory.0.join("agent-room-desktop.exe"));
}

#[cfg(target_os = "macos")]
#[test]
fn 找的是装着自己的应用包_没有就不拉起() {
    let directory = ScratchDirectory::new();
    let loose = directory.0.join("agent-room-mcp");
    assert!(super::InstalledDesktop::beside(&loose).is_none());

    let bundle = directory.0.join("Agent Room.app");
    let binaries = bundle.join("Contents").join("MacOS");
    std::fs::create_dir_all(&binaries).expect("可建目录");
    let desktop =
        super::InstalledDesktop::beside(&binaries.join("agent-room-mcp")).expect("找到应用包");
    assert_eq!(desktop.target, bundle);
}
