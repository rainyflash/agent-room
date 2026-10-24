//! 桌面端发现自己跑在别的打包应用的包里时，在包外重新启动一份，自己退出。
//!
//! 新版 MCP 和命令行拉起桌面端时已经放在包外（`agent-room-windows-app-package`）。这里兜住别的路：
//! 升级以后宿主里还开着的旧版 MCP 和命令行，以及 Agent 在宿主的终端里直接启动桌面端。跑在包里的
//! 桌面端、Bridge 和 `WebView` 会把一部分本机数据写进宿主的私有副本，下次正常启动时就对不上了。

#[cfg(any(windows, test))]
use std::ffi::OsString;

/// 重新启动的那一份带着它。它还在包里（系统不让出去）就不再试、照常在包里跑，免得一直重启。
const RELAUNCHED_ARGUMENT: &str = "--outside-app-package";

/// 在包外起了新的一份时返回 `true`，调用方应当马上退出；不在包里、已经换过或者起不来时返回 `false`。
#[cfg(windows)]
pub(crate) fn relaunched_outside_app_package() -> bool {
    use agent_room_windows_app_package::{inside_app_package, spawn_outside_app_package};

    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some(arguments) = relaunch_arguments(inside_app_package(), &arguments) else {
        return false;
    };
    let Ok(executable) = std::env::current_exe() else {
        return false;
    };
    // 和 Agent 拉起桌面端一样：能脱离宿主的作业就脱离，不允许时照常启动。
    spawn_outside_app_package(&executable, &arguments, CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| spawn_outside_app_package(&executable, &arguments, 0))
        .is_ok()
}

/// 只有 Windows 有打包应用的包。
#[cfg(not(windows))]
pub(crate) fn relaunched_outside_app_package() -> bool {
    false
}

/// 写进桌面端日志：这一份是从包里换出来的，或者没换成、还在包里。
pub(crate) fn log_launch_context() {
    let relaunched = std::env::args_os().any(|argument| argument == RELAUNCHED_ARGUMENT);
    if inside_package() {
        tracing::warn!(
            relaunched,
            "桌面端跑在别的应用的包里，本机数据可能有一部分写进那个应用的私有副本"
        );
    } else if relaunched {
        tracing::info!("桌面端是在别的应用的包里被拉起的，已在包外重新启动");
    }
}

#[cfg(windows)]
fn inside_package() -> bool {
    agent_room_windows_app_package::inside_app_package()
}

#[cfg(not(windows))]
fn inside_package() -> bool {
    false
}

/// 要在包外重新启动时，给出新的一份的参数：原来的参数后面加上 `RELAUNCHED_ARGUMENT`。
#[cfg(any(windows, test))]
fn relaunch_arguments(inside_package: bool, arguments: &[OsString]) -> Option<Vec<OsString>> {
    if !inside_package
        || arguments
            .iter()
            .any(|argument| argument == RELAUNCHED_ARGUMENT)
    {
        return None;
    }
    let mut relaunched = arguments.to_vec();
    relaunched.push(RELAUNCHED_ARGUMENT.into());
    Some(relaunched)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{RELAUNCHED_ARGUMENT, relaunch_arguments};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn 不在包里就照常启动() {
        assert_eq!(
            relaunch_arguments(false, &arguments(&["--background"])),
            None
        );
    }

    #[test]
    fn 在包里就带着原来的参数到包外重新启动() {
        assert_eq!(
            relaunch_arguments(true, &arguments(&["--background", "agentroom://join/abc"])),
            Some(arguments(&[
                "--background",
                "agentroom://join/abc",
                RELAUNCHED_ARGUMENT
            ]))
        );
    }

    #[test]
    fn 换过一次还在包里就不再试() {
        assert_eq!(
            relaunch_arguments(true, &arguments(&["--background", RELAUNCHED_ARGUMENT])),
            None
        );
    }

    #[test]
    fn 后台启动和链接都认得重新启动的参数() {
        let relaunched = relaunch_arguments(true, &arguments(&["--background"]))
            .expect("在包里要重新启动")
            .into_iter()
            .map(|argument| argument.into_string().expect("参数是文字"))
            .collect::<Vec<_>>();
        assert!(crate::launched_in_background(relaunched.iter().cloned()));
        assert!(
            crate::deep_link::deep_links_in_arguments(relaunched.iter().map(String::as_str))
                .is_empty()
        );
        assert_eq!(
            crate::installer_acceptance::launch_mode(relaunched.into_iter().map(OsString::from)),
            crate::installer_acceptance::DesktopLaunchMode::Interactive
        );
    }
}
