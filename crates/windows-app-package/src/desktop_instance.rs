//! 同一时间只让一份桌面端真正跑起来：桌面端排队用的具名对象，命令行和 MCP 也拿它看桌面端在不在。
//!
//! - 正在跑的那份桌面端一直拿着 [`PRIMARY_MUTEX`]，进程结束（正常退出或被结束）时系统替它放开；
//! - 它开始退出时建 [`CLOSING_EVENT`] 并开着到进程结束，新启动的桌面端看到它就等它退完，
//!   不把参数交给一个马上要走的进程。
//!
//! 名字都在当前登录会话的命名空间（`Local\`）里，同一个会话里的进程看到的是同一个对象。

use std::io;

use crate::ffi::{self, OwnedHandle};

/// 正在跑的那份桌面端拿着的互斥体。
pub const PRIMARY_MUTEX: &str = r"Local\dev.agent-room.desktop.primary";
/// 正在跑的那份桌面端开始退出时建的事件。
pub const CLOSING_EVENT: &str = r"Local\dev.agent-room.desktop.closing";

/// 排队用的互斥体句柄。拿到以后用 [`PrimaryClaim::hold_until_exit`] 一直留着。
pub struct PrimaryClaim(OwnedHandle);

impl PrimaryClaim {
    /// 打开（没有就建）排队用的互斥体，还不拿。
    ///
    /// # Errors
    ///
    /// 系统建不了这个互斥体时返回错误。
    pub fn open() -> io::Result<Self> {
        Self::open_named(PRIMARY_MUTEX)
    }

    fn open_named(name: &str) -> io::Result<Self> {
        ffi::create_mutex(&wide(name)).map(Self)
    }

    /// 不等，试着拿：拿到了（或者上一份桌面端没放就结束了）返回 `true`。
    ///
    /// 拿到的是调用它的线程，要在桌面端的主线程上调用。
    ///
    /// # Errors
    ///
    /// 系统等待失败时返回错误。
    pub fn try_take(&self) -> io::Result<bool> {
        ffi::try_lock(&self.0)
    }

    /// 拿到以后调用：句柄留到进程结束，由系统在进程结束时放开。
    pub fn hold_until_exit(self) {
        std::mem::forget(self);
    }
}

/// 有一份桌面端在跑，而且没在退出。
///
/// 只认得这个版本起的桌面端；更早的版本不拿 [`PRIMARY_MUTEX`]，在跑也返回 `false`。
pub fn desktop_running() -> bool {
    ffi::mutex_exists(&wide(PRIMARY_MUTEX)) && !closing_announced()
}

/// 有没有进程开着这个名字的互斥体（比如单实例插件的那个）。
pub fn mutex_exists(name: &str) -> bool {
    ffi::mutex_exists(&wide(name))
}

/// 有一份桌面端说它正在退出。
pub fn closing_announced() -> bool {
    ffi::event_exists(&wide(CLOSING_EVENT))
}

/// 说这份桌面端正在退出。放下时撤回（同一会话里没有别的进程也在说的话）。
pub struct ClosingAnnouncement {
    // 只靠开着它说话，放下时关掉。
    _event: OwnedHandle,
}

impl ClosingAnnouncement {
    /// 一直说到进程结束。
    pub fn keep_until_exit(self) {
        std::mem::forget(self);
    }
}

/// 说这份桌面端正在退出。
///
/// # Errors
///
/// 系统建不了这个事件时返回错误。
pub fn announce_closing() -> io::Result<ClosingAnnouncement> {
    announce_named(CLOSING_EVENT)
}

fn announce_named(name: &str) -> io::Result<ClosingAnnouncement> {
    ffi::create_event(&wide(name)).map(|event| ClosingAnnouncement { _event: event })
}

/// 把 `data` 交给类名和标题都对得上的窗口（`WM_COPYDATA`，`kind` 填进 `dwData`）。
/// 找不到窗口、对方卡住或者 `timeout_ms` 内没收下时返回 `false`。
pub fn send_to_window(class: &str, title: &str, kind: usize, data: &[u8], timeout_ms: u32) -> bool {
    ffi::send_copy_data(&wide(class), &wide(title), kind, data, timeout_ms)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{PrimaryClaim, announce_named, ffi, send_to_window, wide};

    /// 测试用自己的名字：开发机上可能正开着真的桌面端。
    fn test_name(purpose: &str) -> String {
        format!(r"Local\agent-room-test-{purpose}-{}", std::process::id())
    }

    #[test]
    fn 有线程拿着时别人拿不到_它结束后下一个拿得到() {
        let name = test_name("claim");
        let (taken, wait_taken) = mpsc::channel();
        let (release, wait_release) = mpsc::channel::<()>();
        let holder_name = name.clone();
        let holder = std::thread::spawn(move || {
            let claim = PrimaryClaim::open_named(&holder_name).expect("能建互斥体");
            assert!(claim.try_take().expect("能等"));
            // 互斥体可重入：同一个线程再拿一次照样拿到。
            assert!(claim.try_take().expect("能等"));
            taken.send(()).expect("测试线程在等");
            let _ = wait_release.recv();
            // 不放就结束线程，和桌面端进程结束一样由系统放开。
            claim.hold_until_exit();
        });
        wait_taken.recv().expect("拿着它的线程报了信");

        let other = PrimaryClaim::open_named(&name).expect("能打开互斥体");
        assert!(!other.try_take().expect("能等"), "有线程拿着时别人拿不到");

        release.send(()).expect("拿着它的线程在等");
        holder.join().expect("线程正常结束");
        assert!(other.try_take().expect("能等"), "上一个线程结束后能拿到");
    }

    #[test]
    fn 说了正在退出以后看得到_撤回以后看不到() {
        let name = test_name("closing");
        assert!(!ffi::event_exists(&wide(&name)));
        let announcement = announce_named(&name).expect("能建事件");
        assert!(ffi::event_exists(&wide(&name)));
        drop(announcement);
        assert!(!ffi::event_exists(&wide(&name)));

        announce_named(&name).expect("能建事件").keep_until_exit();
        assert!(ffi::event_exists(&wide(&name)), "留到进程结束");
    }

    #[test]
    fn 找不到窗口时交不出去() {
        assert!(!send_to_window(
            "agent-room-test-no-such-class",
            "agent-room-test-no-such-window",
            1,
            b"x",
            100,
        ));
    }
}
