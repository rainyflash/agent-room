use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

use crate::desktop_config::DesktopBridgeConfig;

const INSTALLER_ACCEPTANCE_ARGUMENT: &str = "--installer-acceptance";
const INSTALLER_VERSION_ARGUMENT: &str = "--installer-version";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopLaunchMode {
    Interactive,
    InstallerAcceptance,
    InstallerVersion,
}

pub(crate) fn launch_mode(arguments: impl IntoIterator<Item = OsString>) -> DesktopLaunchMode {
    for argument in arguments {
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
    let config = DesktopBridgeConfig::from_environment()
        .map_err(|_| InstallerAcceptanceFailure::new("desktop.acceptance.config_invalid"))?;

    let status = Command::new(bridge)
        .current_dir(directory)
        .envs(config.environment())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| InstallerAcceptanceFailure::new("desktop.acceptance.bridge_spawn_failed"))?
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

    use super::{DesktopLaunchMode, launch_mode};

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
}
