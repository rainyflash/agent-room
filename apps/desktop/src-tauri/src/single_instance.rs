//! Windows 上同一时间只让一份桌面端跑起来；别的启动把参数交给它就退出。
//!
//! 单实例插件（`tauri-plugin-single-instance`）只看它的具名互斥体在不在：在的话去找正在跑的那份
//! 收消息的窗口，把参数交给它；找不到窗口时它不拦，当成没人在跑。上一份正在退出（窗口关了、
//! 互斥体还没放）或者刚启动（互斥体建了、窗口还没建）时碰上，两份就一起跑起来了；插件这时还开着
//! 自己的互斥体不放，之后每次启动都找不到窗口，一直多起下去。两份同时打开同一份 `WebView` 本机数据，
//! 后起的那份主窗口的 `WebView` 建不起来，只剩托盘图标、怎么点都出不来（2026-10-25 维护者电脑上：
//! 发版验收后十几个 Agent 的 MCP 和命令行同时把桌面端拉起来，正好碰上他自己点开）。
//!
//! 所以在插件之前先排队（`agent_room_windows_app_package::desktop_instance`）：
//! - 拿到排队的互斥体就是正在跑的那一份，等上一份的 `WebView` 放开本机数据再往下启动；
//! - 拿不到，对方没在退出、收消息的窗口也在：照插件的格式把参数交给它，自己退出；
//! - 对方正在退出，或者它的窗口还没建好：过一会儿再看，直到拿到或者交出去。
//!
//! 收参数照旧是插件。排队以后只有拿到互斥体的那一份会走到插件，插件的互斥体总是新建的。

#[cfg(any(windows, test))]
use std::time::{Duration, Instant};

/// 插件按应用标识建收消息的窗口：类名 `{标识}-sic`、标题 `{标识}-siw`（插件 2.4.3 的 `windows.rs`）。
/// 和 `tauri.conf.json` 的 `identifier` 一样，测试卡着。
#[cfg(any(windows, test))]
const APP_IDENTIFIER: &str = "dev.agent-room.desktop";
/// 插件收参数时只认这个 `dwData`。
#[cfg(windows)]
const FORWARD_KIND: usize = 1542;
/// 对方卡住时不一直等。
#[cfg(windows)]
const FORWARD_TIMEOUT_MS: u32 = 10_000;
/// 上一份退出时要等托管的 Bridge 有序退出（最多十来秒），留足余量。
#[cfg(windows)]
const TURN_TIMEOUT: Duration = Duration::from_mins(1);
#[cfg(any(windows, test))]
const TURN_POLL: Duration = Duration::from_millis(100);
/// 被结束的上一份关句柄只要一眨眼；等不到也照常启动。
#[cfg(windows)]
const PLUGIN_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
/// 上一份的 `WebView` 浏览器进程会比它多活一会儿；等不到也照常启动。
#[cfg(windows)]
const WEBVIEW_RELEASE_TIMEOUT: Duration = Duration::from_secs(15);
/// 主窗口的 `WebView` 没建起来、重新启动的那一份带着它，只重来一次。
#[cfg(windows)]
const RETRY_ARGUMENT: &str = "--retry-main-window";

/// 排队时看到的情况。真的实现在下面，测试里换成假的。
#[cfg(any(windows, test))]
trait Instances {
    /// 拿到排队的互斥体时返回 `true`。
    fn try_become_primary(&mut self) -> bool;
    /// 正在跑的那一份说它正在退出。
    fn closing(&self) -> bool;
    /// 把这次启动的参数交给正在跑的那一份；找不到它收消息的窗口或者它没收下时返回 `false`。
    fn forward(&self) -> bool;
}

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
enum Turn {
    Primary,
    Forwarded,
    TimedOut,
}

#[cfg(any(windows, test))]
fn wait_for_turn(
    instances: &mut impl Instances,
    timeout: Duration,
    mut pause: impl FnMut(Duration),
) -> Turn {
    let deadline = Instant::now() + timeout;
    loop {
        if instances.try_become_primary() {
            return Turn::Primary;
        }
        // 正在退出的那一份马上就走，交给它等于白交：等它退完自己接上。
        if !instances.closing() && instances.forward() {
            return Turn::Forwarded;
        }
        if Instant::now() >= deadline {
            return Turn::TimedOut;
        }
        pause(TURN_POLL);
    }
}

/// 插件收参数的格式：`{工作目录}|{参数1}|{参数2}…`，以 NUL 结尾，参数里包括程序本身。
#[cfg(any(windows, test))]
fn forward_payload(cwd: &str, arguments: &[String]) -> Vec<u8> {
    format!("{cwd}|{}\0", arguments.join("|")).into_bytes()
}

/// 在建 Tauri 应用之前、在主线程上调用。轮到这一份时返回 `true`；交给了正在跑的那一份，
/// 或者等不到又是后台启动时返回 `false`，调用方直接退出。
#[cfg(windows)]
pub(crate) fn wait_for_turn_to_run(background: bool) -> bool {
    use agent_room_windows_app_package::desktop_instance::PrimaryClaim;

    let Ok(claim) = PrimaryClaim::open() else {
        // 排不了队就和以前一样，交给插件。
        return true;
    };
    let mut instances = SystemInstances {
        claim: Some(claim),
        payload: current_payload(),
    };
    match wait_for_turn(&mut instances, TURN_TIMEOUT, std::thread::sleep) {
        Turn::Primary => {
            wait_for_previous_to_let_go();
            true
        }
        Turn::Forwarded => false,
        Turn::TimedOut => {
            install_log();
            tracing::warn!(
                background,
                "等了一分钟还没轮到这份桌面端：正在跑的那份既没交出位置，也收不了参数"
            );
            // Agent 拉起的后台启动下次再来；人点开的照常启动，至少看得到窗口。
            !background
        }
    }
}

/// 只有 Windows 需要排队：macOS 由系统保证一个应用只开一份。
#[cfg(not(windows))]
pub(crate) fn wait_for_turn_to_run(_background: bool) -> bool {
    true
}

#[cfg(windows)]
struct SystemInstances {
    claim: Option<agent_room_windows_app_package::desktop_instance::PrimaryClaim>,
    payload: Vec<u8>,
}

#[cfg(windows)]
impl Instances for SystemInstances {
    fn try_become_primary(&mut self) -> bool {
        let taken = self
            .claim
            .as_ref()
            .is_some_and(|claim| claim.try_take().unwrap_or(false));
        if taken && let Some(claim) = self.claim.take() {
            claim.hold_until_exit();
        }
        taken
    }

    fn closing(&self) -> bool {
        agent_room_windows_app_package::desktop_instance::closing_announced()
    }

    fn forward(&self) -> bool {
        agent_room_windows_app_package::desktop_instance::send_to_window(
            &format!("{APP_IDENTIFIER}-sic"),
            &format!("{APP_IDENTIFIER}-siw"),
            FORWARD_KIND,
            &self.payload,
            FORWARD_TIMEOUT_MS,
        )
    }
}

#[cfg(windows)]
fn current_payload() -> Vec<u8> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let arguments: Vec<String> = std::env::args_os()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    forward_payload(&cwd.to_string_lossy(), &arguments)
}

/// 轮到这一份时上一份的线程都结束了，但它可能还没走完：
/// - 被直接结束的进程先放开排队的互斥体，再关别的句柄。插件的互斥体还开着的话，这一份的插件会当
///   有人在跑、不建收消息的窗口，之后的启动就都交不进来；
/// - `WebView` 的浏览器进程开着本机数据时一直占着 `EBWebView\lockfile`（和安装器钩子查的是同一个），
///   它会比桌面端多活一会儿，这时建 `WebView` 会失败。
///
/// 都等一会儿，等不到也照常启动。
#[cfg(windows)]
fn wait_for_previous_to_let_go() {
    use agent_room_windows_app_package::desktop_instance::mutex_exists;

    let plugin_mutex = format!("{APP_IDENTIFIER}-sim");
    wait_while(PLUGIN_RELEASE_TIMEOUT, || mutex_exists(&plugin_mutex));
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let lockfile = std::path::Path::new(&local)
        .join(APP_IDENTIFIER)
        .join("EBWebView")
        .join("lockfile");
    wait_while(WEBVIEW_RELEASE_TIMEOUT, || webview_data_in_use(&lockfile));
}

#[cfg(windows)]
fn wait_while(timeout: Duration, mut busy: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 照安装器钩子的办法：以写、只共享读的方式打开已有的文件，共享冲突或锁冲突就是还被占着。
#[cfg(windows)]
fn webview_data_in_use(lockfile: &std::path::Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_SHARE_READ: u32 = 0x1;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    match std::fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ)
        .open(lockfile)
    {
        Ok(_) => false,
        Err(error) => matches!(
            error.raw_os_error(),
            Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
        ),
    }
}

/// 主窗口的 `WebView` 没建起来时，Tauri 只记一行日志、照样往下跑：这份就只剩托盘，点“打开”也出不来。
/// 这时说自己正在退出、另起一份（它等这份退完、`WebView` 放开本机数据才往下走），自己马上退出。
/// 只重来一次；重来的那份还不行，就照常跑着，至少 Bridge 和托盘在。要在启动 Bridge 之前调用。
#[cfg(windows)]
pub(crate) fn recover_missing_main_window(app: &tauri::App) {
    use tauri::Manager as _;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.hwnd().is_ok() {
        return;
    }
    install_log();
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if arguments.iter().any(|argument| argument == RETRY_ARGUMENT) {
        tracing::error!("主窗口重来一次还是没建起来，这份桌面端只有托盘");
        return;
    }
    if relaunch_with_retry(arguments) {
        tracing::warn!("主窗口没建起来，另起一份桌面端再试一次");
        std::process::exit(0);
    }
    tracing::error!("主窗口没建起来，也没能另起一份，这份桌面端只有托盘");
}

#[cfg(windows)]
fn relaunch_with_retry(mut arguments: Vec<std::ffi::OsString>) -> bool {
    use agent_room_windows_app_package::{desktop_instance, spawn_outside_app_package};

    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let Ok(executable) = std::env::current_exe() else {
        return false;
    };
    // 先说正在退出，新的一份才不会把参数交给这一份；没起成就撤回，这一份接着跑。
    let Ok(closing) = desktop_instance::announce_closing() else {
        return false;
    };
    arguments.push(RETRY_ARGUMENT.into());
    let spawned = spawn_outside_app_package(&executable, &arguments, CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| spawn_outside_app_package(&executable, &arguments, 0))
        .is_ok();
    if spawned {
        closing.keep_until_exit();
    }
    spawned
}

/// 只有 Windows 的 `WebView2` 会这样。
#[cfg(not(windows))]
pub(crate) fn recover_missing_main_window(_app: &tauri::App) {}

/// 开始退出时调用：新启动的桌面端看到就等这一份退完，不把参数交给它。
pub(crate) fn announce_closing() {
    #[cfg(windows)]
    if let Ok(closing) = agent_room_windows_app_package::desktop_instance::announce_closing() {
        closing.keep_until_exit();
    }
}

/// 排队前桌面端的日志还没装上；要写一笔时先装上。
#[cfg(windows)]
fn install_log() {
    if let Ok(config) = crate::desktop_config::DesktopBridgeConfig::from_environment() {
        crate::logging::LogLocation::new(&config.data_root()).install();
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, time::Duration};

    use super::{APP_IDENTIFIER, Instances, Turn, forward_payload, wait_for_turn};

    /// 按顺序给出每一轮看到的情况。
    #[derive(Default)]
    struct Script {
        primary_at: Option<usize>,
        closing_until: usize,
        window_at: Option<usize>,
        round: Cell<usize>,
        forwarded: Cell<bool>,
    }

    impl Instances for Script {
        fn try_become_primary(&mut self) -> bool {
            let round = self.round.get();
            self.round.set(round + 1);
            self.primary_at.is_some_and(|at| round >= at)
        }

        fn closing(&self) -> bool {
            self.round.get() <= self.closing_until
        }

        fn forward(&self) -> bool {
            let reached = self.window_at.is_some_and(|at| self.round.get() > at);
            self.forwarded.set(reached);
            reached
        }
    }

    fn run(script: &mut Script, timeout: Duration) -> (Turn, usize) {
        let mut pauses = 0;
        let turn = wait_for_turn(script, timeout, |_| pauses += 1);
        (turn, pauses)
    }

    #[test]
    fn 没人在跑时马上轮到() {
        let mut script = Script {
            primary_at: Some(0),
            ..Script::default()
        };
        assert_eq!(run(&mut script, Duration::from_secs(1)), (Turn::Primary, 0));
    }

    #[test]
    fn 有一份在跑时把参数交给它() {
        let mut script = Script {
            window_at: Some(0),
            ..Script::default()
        };
        assert_eq!(
            run(&mut script, Duration::from_secs(1)),
            (Turn::Forwarded, 0)
        );
        assert!(script.forwarded.get());
    }

    #[test]
    fn 对方刚启动_窗口还没建好时等它建好再交() {
        let mut script = Script {
            window_at: Some(3),
            ..Script::default()
        };
        assert_eq!(
            run(&mut script, Duration::from_secs(10)),
            (Turn::Forwarded, 3)
        );
    }

    #[test]
    fn 对方正在退出时不交给它_等它退完自己接上() {
        let mut script = Script {
            closing_until: 5,
            window_at: Some(0),
            primary_at: Some(5),
            ..Script::default()
        };
        assert_eq!(
            run(&mut script, Duration::from_secs(10)),
            (Turn::Primary, 5)
        );
        assert!(!script.forwarded.get(), "正在退出的那份收不到参数");
    }

    #[test]
    fn 既轮不到也交不出去时到点放弃() {
        let mut script = Script::default();
        let (turn, _) = run(&mut script, Duration::ZERO);
        assert_eq!(turn, Turn::TimedOut);
    }

    #[test]
    fn 交出去的参数和插件收的格式一样() {
        let arguments = [
            r"C:\Apps\agent-room-desktop.exe".to_owned(),
            "--background".to_owned(),
        ];
        assert_eq!(
            forward_payload(r"C:\Users\me", &arguments),
            b"C:\\Users\\me|C:\\Apps\\agent-room-desktop.exe|--background\0"
        );
    }

    /// 转交参数照的是插件 2.4.3 的窗口名和格式。升级插件时先核对它的 `platform_impl/windows.rs`，
    /// 再改这里的版本号。
    #[test]
    fn 单实例插件的版本和应用标识没变() {
        let lock = include_str!("../../../../Cargo.lock");
        let mut lines = lock.lines().map(str::trim);
        let version = lines
            .by_ref()
            .find(|line| *line == r#"name = "tauri-plugin-single-instance""#)
            .and_then(|_| lines.next());
        assert_eq!(version, Some(r#"version = "2.4.3""#));

        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("配置是 JSON");
        assert_eq!(config["identifier"], APP_IDENTIFIER);
    }
}
