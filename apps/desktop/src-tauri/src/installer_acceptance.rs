use std::{
    ffi::OsString,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use crate::desktop_config::DesktopBridgeConfig;

const INSTALLER_ACCEPTANCE_ARGUMENT: &str = "--installer-acceptance";
const INSTALLER_VERSION_ARGUMENT: &str = "--installer-version";
/// 安装验收只问一件事：装完之后桌面端能不能拉起本机运行时。
/// 未授权的设备会停在等待批准设备码上，不会自己退出，所以看到运行时起来就收尾。
const RUNTIME_START_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeSignal {
    Started,
    OutputEnded,
}

/// Bridge 在受监督模式下按行输出事件；这三种都表示运行时已经起来了。
fn runtime_started(line: &str) -> bool {
    let event = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("event").and_then(serde_json::Value::as_str).map(str::to_owned));
    matches!(
        event.as_deref(),
        Some("authorization_required" | "device_authorized" | "ready")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopLaunchMode {
    Interactive,
    InstallerAcceptance,
    InstallerVersion,
    HostCheck,
}

pub(crate) fn launch_mode(arguments: impl IntoIterator<Item = OsString>) -> DesktopLaunchMode {
    for argument in arguments {
        if argument == "--check-hosts" {
            return DesktopLaunchMode::HostCheck;
        }
        if argument == INSTALLER_ACCEPTANCE_ARGUMENT {
            return DesktopLaunchMode::InstallerAcceptance;
        }
        if argument == INSTALLER_VERSION_ARGUMENT {
            return DesktopLaunchMode::InstallerVersion;
        }
    }
    DesktopLaunchMode::Interactive
}

pub(crate) fn print_version() -> ExitCode {
    println!(env!("CARGO_PKG_VERSION"));
    ExitCode::SUCCESS
}

pub(crate) fn run() -> ExitCode {
    match run_managed_bridge() {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("Agent Room 安装器验收失败 [{}]", failure.code());
            ExitCode::FAILURE
        }
    }
}

fn run_managed_bridge() -> Result<(), InstallerAcceptanceFailure> {
    let executable = std::env::current_exe().map_err(|_| {
        InstallerAcceptanceFailure::new("desktop.acceptance.executable_unavailable")
    })?;
    let directory = executable
        .parent()
        .ok_or_else(|| InstallerAcceptanceFailure::new("desktop.acceptance.directory_invalid"))?;
    let bridge = installed_runtime_executable(directory, "agent-room-bridge")?;
    let _mcp = installed_runtime_executable(directory, "agent-room-mcp")?;
    let _cli = installed_runtime_executable(directory, "agent-room")?;
    let config = DesktopBridgeConfig::from_environment()
        .map_err(|_| InstallerAcceptanceFailure::new("desktop.acceptance.config_invalid"))?;

    let mut child = Command::new(bridge)
        .current_dir(directory)
        .envs(config.environment())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| InstallerAcceptanceFailure::new("desktop.acceptance.bridge_spawn_failed"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| InstallerAcceptanceFailure::new("desktop.acceptance.bridge_output_missing"))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if runtime_started(&line) {
                let _ = sender.send(BridgeSignal::Started);
                return;
            }
        }
        let _ = sender.send(BridgeSignal::OutputEnded);
    });

    let signal = receiver.recv_timeout(RUNTIME_START_TIMEOUT);
    if signal == Ok(BridgeSignal::Started) {
        // 运行时已经起来：验收到此为止，由桌面端收走自己拉起的 Bridge。
        let _ = child.kill();
        let _ = child.wait();
        return Ok(());
    }
    if signal == Err(RecvTimeoutError::Timeout) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(InstallerAcceptanceFailure::new(
            "desktop.acceptance.bridge_not_ready",
        ));
    }
    // Bridge 先结束了输出：沿用原来的判断，退出码不为 0 就是失败。
    let status = child
        .wait()
        .map_err(|_| InstallerAcceptanceFailure::new("desktop.acceptance.bridge_wait_failed"))?;
    if status.success() {
        Ok(())
    } else {
        Err(InstallerAcceptanceFailure::new(
            "desktop.acceptance.bridge_exited",
        ))
    }
}

fn installed_runtime_executable(
    directory: &Path,
    basename: &str,
) -> Result<PathBuf, InstallerAcceptanceFailure> {
    let path = directory.join(format!("{basename}{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(InstallerAcceptanceFailure::new(
            "desktop.acceptance.runtime_missing",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InstallerAcceptanceFailure {
    code: &'static str,
}

impl InstallerAcceptanceFailure {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }

    const fn code(self) -> &'static str {
        self.code
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{DesktopLaunchMode, launch_mode, runtime_started};

    #[test]
    fn 运行时起来的三种事件都算安装验收通过() {
        for event in ["authorization_required", "device_authorized", "ready"] {
            assert!(runtime_started(&format!(r#"{{"event":"{event}","channel":"agent_room_desktop"}}"#)));
        }
    }

    #[test]
    fn 还没起来的事件与非事件输出不算通过() {
        // 连不上服务时 Bridge 会保持运行并重试，这不是「已经起来」。
        assert!(!runtime_started(
            r#"{"event":"server_unreachable","channel":"agent_room_desktop","code":"bridge.identity_provider_unavailable","retryAfterMs":5000}"#
        ));
        assert!(!runtime_started(
            r#"{"event":"transient_failure","channel":"agent_room_desktop","code":"bridge.agent_online_failed"}"#
        ));
        assert!(!runtime_started("不是 JSON 的一行"));
        assert!(!runtime_started(""));
    }

    #[test]
    fn 精确参数进入无_webview_安装器验收模式() {
        assert_eq!(
            launch_mode([OsString::from("--installer-acceptance")]),
            DesktopLaunchMode::InstallerAcceptance
        );
        assert_eq!(
            launch_mode([OsString::from("--installer-acceptance=true")]),
            DesktopLaunchMode::Interactive
        );
    }

    #[test]
    fn 普通启动与自启动保持交互模式() {
        assert_eq!(
            launch_mode(std::iter::empty()),
            DesktopLaunchMode::Interactive
        );
        assert_eq!(
            launch_mode([OsString::from("--autostart")]),
            DesktopLaunchMode::Interactive
        );
    }

    #[test]
    fn 精确参数进入版本探测模式() {
        assert_eq!(
            launch_mode([OsString::from("--installer-version")]),
            DesktopLaunchMode::InstallerVersion
        );
        assert_eq!(
            launch_mode([OsString::from("--installer-version=true")]),
            DesktopLaunchMode::Interactive
        );
    }

    #[test]
    fn host_check_is_an_explicit_headless_entrypoint() {
        assert_eq!(
            launch_mode([OsString::from("--check-hosts")]),
            DesktopLaunchMode::HostCheck
        );
        assert_eq!(
            launch_mode([OsString::from("--check-hosts=true")]),
            DesktopLaunchMode::Interactive
        );
    }
}
