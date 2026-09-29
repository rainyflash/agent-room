//! 本机 Bridge 不在时，把装在一起的桌面端在后台拉起来。
//!
//! Bridge 由桌面端启动和监督。桌面端被关掉、或者重启电脑后没开，Agent 的 MCP 与命令行就连不上，
//! 只能报“启动 Agent Room Bridge”——Agent 自己做不到，用户也看不见。这里在确认 Bridge 不在时，
//! 以不弹窗口的方式启动桌面端，等 Bridge 应答后把这次调用重做一遍。

use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use agent_room_bridge_ipc::{IpcBridgeState, IpcMethod, IpcResponse};
use agent_room_bridge_local_adapter::{SecureStorageService, resolve_bridge_data_root};
use tokio::time::Instant;

use crate::bridge::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture, LocalBridgeToolClient};

/// MCP 与命令行用：连的是桌面端那一套数据目录和凭据命名空间、旁边也装着桌面端时，
/// 连不上 Bridge 就把桌面端在后台拉起来；否则原样连。
///
/// 发布验收、隔离测试用自己的命名空间：拉起正式桌面端既帮不上它们，还会打扰用户。
/// 开发构建也不拉：`target` 目录里挨着的是调试版桌面端，它会改写注册表里的链接处理程序。
#[must_use]
pub fn launching_desktop_when_absent(
    client: LocalBridgeToolClient,
    data_root: &Path,
    service: &SecureStorageService,
) -> Arc<dyn BridgeToolClient> {
    let desktop = if cfg!(debug_assertions) {
        None
    } else {
        InstalledDesktop::for_namespace(data_root, service)
    };
    match desktop {
        Some(desktop) => Arc::new(DesktopLaunchingClient::new(client, desktop)),
        None => Arc::new(client),
    }
}

/// 带这个参数启动的桌面端不弹主窗口，只在托盘里跑（桌面端 `lib.rs` 的 `launched_in_background`）。
pub const DESKTOP_BACKGROUND_ARGUMENT: &str = "--background";
/// 同一个进程两次拉起之间至少隔这么久：桌面端起不来时不反复启动。
const RELAUNCH_INTERVAL: Duration = Duration::from_mins(2);
/// 拉起之后等 Bridge 就绪的上限。
const READY_TIMEOUT: Duration = Duration::from_mins(1);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// 启动桌面端的方式；测试里换成假的。
pub trait DesktopLauncher: Send + Sync + 'static {
    /// 在后台启动桌面端。桌面端已经在运行时，它会自己把这次启动并到已有的实例上。
    ///
    /// # Errors
    ///
    /// 桌面端程序不存在或无法启动时返回错误。
    fn launch(&self) -> io::Result<()>;
}

/// 和当前程序装在一起的桌面端。
#[derive(Debug, Clone)]
pub struct InstalledDesktop {
    target: PathBuf,
}

impl InstalledDesktop {
    /// 只在用的是桌面端默认命名空间时，才找和当前程序装在一起的桌面端。
    #[must_use]
    pub fn for_namespace(data_root: &Path, service: &SecureStorageService) -> Option<Self> {
        let desktop_root = resolve_bridge_data_root(|name| match name {
            "AGENT_ROOM_BRIDGE_DATA_DIR" => None,
            _ => std::env::var(name).ok(),
        })
        .ok()?;
        if data_root != desktop_root || *service != SecureStorageService::default() {
            return None;
        }
        Self::beside_current_executable()
    }

    /// 找和当前程序装在一起的桌面端；开发构建、Linux 运行时镜像这类没有桌面端的环境返回 `None`。
    #[must_use]
    pub fn beside_current_executable() -> Option<Self> {
        let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
        Self::beside(&executable)
    }

    fn beside(executable: &Path) -> Option<Self> {
        desktop_target(executable)
            .filter(|target| target.exists())
            .map(|target| Self { target })
    }
}

/// Windows：安装目录里与 MCP、命令行放在一起的桌面端程序。
#[cfg(windows)]
fn desktop_target(executable: &Path) -> Option<PathBuf> {
    Some(executable.parent()?.join("agent-room-desktop.exe"))
}

/// macOS：MCP 与命令行在应用包的 `Contents/MacOS` 里，启动的是整个应用包。
#[cfg(target_os = "macos")]
fn desktop_target(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf)
}

/// 其他系统上没有桌面端，只有自己部署的运行时。
#[cfg(not(any(windows, target_os = "macos")))]
fn desktop_target(_executable: &Path) -> Option<PathBuf> {
    None
}

impl DesktopLauncher for InstalledDesktop {
    fn launch(&self) -> io::Result<()> {
        launch_detached(&self.target)
    }
}

/// 桌面端不能继承 MCP 的标准输入输出（那是 MCP 协议的通道），也不能随启动它的宿主一起被结束。
#[cfg(windows)]
fn launch_detached(target: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let spawn = |flags: u32| {
        Command::new(target)
            .arg(DESKTOP_BACKGROUND_ARGUMENT)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
            .map(drop)
    };
    // 宿主可能把 MCP 放在“宿主退出就全部结束”的作业里；能脱离就脱离，不允许脱离时照常启动。
    spawn(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| spawn(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP))
}

/// 经系统启动应用包，`-g` 不把它拉到前台；已经在运行时不会再开一份。
#[cfg(target_os = "macos")]
fn launch_detached(target: &Path) -> io::Result<()> {
    use std::process::{Command, Stdio};

    let status = Command::new("/usr/bin/open")
        .arg("-g")
        .arg("-a")
        .arg(target)
        .arg("--args")
        .arg(DESKTOP_BACKGROUND_ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("open 没能启动 Agent Room"))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn launch_detached(_target: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// 连不上 Bridge 时先把桌面端拉起来再重做这次调用的客户端。
pub struct DesktopLaunchingClient<C, L> {
    inner: C,
    launcher: L,
    last_launch: Mutex<Option<Instant>>,
}

impl<C, L> DesktopLaunchingClient<C, L> {
    pub const fn new(inner: C, launcher: L) -> Self {
        Self {
            inner,
            launcher,
            last_launch: Mutex::new(None),
        }
    }
}

/// 这次调用失败时要不要去拉起桌面端。
enum LaunchDecision {
    /// 刚拉起（或另一个调用刚拉起）、还在启动窗口里：等 Bridge 就绪到这个时刻。
    WaitUntil(Instant),
    /// 不拉：没到重试间隔，或者启动失败了。
    GiveUp,
}

impl<C: BridgeToolClient, L: DesktopLauncher> DesktopLaunchingClient<C, L> {
    fn decide_launch(&self) -> LaunchDecision {
        let now = Instant::now();
        let mut last = self
            .last_launch
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match *last {
            // 刚拉起过：还在等它启动的窗口里就一起等，过了窗口还没起来就别再等了。
            Some(at) if now.duration_since(at) < READY_TIMEOUT => {
                LaunchDecision::WaitUntil(at + READY_TIMEOUT)
            }
            Some(at) if now.duration_since(at) < RELAUNCH_INTERVAL => LaunchDecision::GiveUp,
            _ => {
                *last = Some(now);
                if self.launcher.launch().is_ok() {
                    LaunchDecision::WaitUntil(now + READY_TIMEOUT)
                } else {
                    LaunchDecision::GiveUp
                }
            }
        }
    }

    /// 等到 Bridge 应答并且就绪。它报离线（比如这台电脑要重新授权）就不再干等；
    /// 报正在关闭多半是上一个 Bridge 还没退完，接着等新的。
    async fn wait_until_ready(&self, deadline: Instant) {
        while Instant::now() < deadline {
            match self.inner.invoke(IpcMethod::BridgeStatus).await {
                Ok(IpcResponse::BridgeStatus {
                    state: IpcBridgeState::Ready | IpcBridgeState::Offline,
                    ..
                }) => return,
                _ => tokio::time::sleep(READY_POLL_INTERVAL).await,
            }
        }
    }
}

impl<C: BridgeToolClient, L: DesktopLauncher> BridgeToolClient for DesktopLaunchingClient<C, L> {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        Box::pin(async move {
            let failure = match self.inner.invoke(method.clone()).await {
                Err(failure) if bridge_absent(&failure) => failure,
                result => return result,
            };
            match self.decide_launch() {
                LaunchDecision::WaitUntil(deadline) => {
                    self.wait_until_ready(deadline).await;
                    self.inner.invoke(method).await
                }
                LaunchDecision::GiveUp => Err(failure),
            }
        })
    }
}

/// Bridge 没在运行：连不上本机端点，或者它还从没初始化过本机凭据。
fn bridge_absent(failure: &BridgeToolFailure) -> bool {
    matches!(
        failure.code(),
        "bridge.ipc.bridge_unavailable" | "bridge.ipc.credentials_missing"
    )
}

#[cfg(test)]
mod tests;
