//! 应用开着时定时查更新（`specs/desktop-update/design.md`）。
//!
//! 关窗只是藏进托盘，应用能一连开好几天；只在页面加载时查一次的话，之后发的版本就查不到了。
//! 这里放不碰网络、不碰界面的部分：什么时候该查、上次检查的结果、每个版本只提醒一次、
//! 提醒和托盘菜单的说法、应用是不是在只读的位置运行。

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use serde::Serialize;

use crate::{
    native_language::NativeLanguage,
    release_updates::{ReleaseUpdateCheck, ReleaseUpdateFailure},
};

/// 应用开着时多久查一次。
pub(crate) const AUTOMATIC_INTERVAL: Duration = Duration::from_hours(4);
/// 窗口重新打开时，离上次检查超过这么久就马上再查。
pub(crate) const WINDOW_RECHECK_AFTER: Duration = Duration::from_hours(1);
/// 后台启动（窗口不出来）以后，先让 Bridge 连上，再查第一次。
pub(crate) const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
/// 定时任务多久醒一次。按墙上时间判断满没满间隔，电脑睡了一夜，醒来这么久之内就会查。
pub(crate) const SCHEDULE_TICK: Duration = Duration::from_mins(5);

/// 离上次检查满没满 `interval`。还没查过算满；墙上时间往回跳了（改过时钟）也算满，
/// 查一次以后就按新的时间算。
pub(crate) fn due(last_attempt: Option<SystemTime>, now: SystemTime, interval: Duration) -> bool {
    last_attempt.is_none_or(|last| {
        now.duration_since(last)
            .map_or(true, |elapsed| elapsed >= interval)
    })
}

/// 上次检查的结果，给界面看：什么时候、按哪个渠道查的，最近一次查成的结果，以及最近这次没查成的原因。
/// 没查成时留着上一次查成的结果：网络断一下，已经知道的新版本不该跟着消失。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseUpdateStatus {
    checked_at_unix_ms: u64,
    channel: &'static str,
    check: Option<ReleaseUpdateCheck>,
    failure: Option<ReleaseUpdateStatusFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseUpdateStatusFailure {
    code: &'static str,
    retryable: bool,
}

impl ReleaseUpdateStatus {
    pub(crate) fn after(
        previous: Option<&Self>,
        channel: &'static str,
        outcome: &Result<ReleaseUpdateCheck, ReleaseUpdateFailure>,
        checked_at_unix_ms: u64,
    ) -> Self {
        match outcome {
            Ok(check) => Self {
                checked_at_unix_ms,
                channel,
                check: Some(check.clone()),
                failure: None,
            },
            Err(failure) => Self {
                checked_at_unix_ms,
                channel,
                check: previous.and_then(|status| status.check.clone()),
                failure: Some(ReleaseUpdateStatusFailure {
                    code: failure.code(),
                    retryable: failure.retryable(),
                }),
            },
        }
    }

    /// 已知可以装的新版本号。
    pub(crate) fn available_version(&self) -> Option<&str> {
        self.check
            .as_ref()
            .filter(|check| check.available())
            .map(ReleaseUpdateCheck::target_version)
    }
}

/// 记住提醒过哪个版本：每个版本只发一次系统通知，重启以后也不再发同一个。
pub(crate) struct UpdateNotice {
    marker: PathBuf,
}

impl UpdateNotice {
    pub(crate) fn new(marker: PathBuf) -> Self {
        Self { marker }
    }

    /// 这个版本还没提醒过时返回 `true`，同时记下。记不住的话下次最多再提醒一回，不算失败。
    pub(crate) fn claim(&self, version: &str) -> bool {
        if std::fs::read_to_string(&self.marker).is_ok_and(|seen| seen.trim() == version) {
            return false;
        }
        if let Some(parent) = self.marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.marker, format!("{version}\n"));
        true
    }
}

/// 查到新版本时的系统通知。
pub(crate) fn available_notification(language: NativeLanguage, version: &str) -> (String, String) {
    match language {
        NativeLanguage::English => (
            "Agent Room update available".to_owned(),
            format!(
                "{version} is ready to install. Open Agent Room and choose “Update and restart”."
            ),
        ),
        NativeLanguage::Chinese => (
            "Agent Room 有新版本".to_owned(),
            format!("{version} 可以安装了。打开 Agent Room，点“更新并重启”。"),
        ),
    }
}

/// 有新版本时托盘菜单里多出来的那一项。
pub(crate) fn tray_update_label(language: NativeLanguage, version: &str) -> String {
    match language {
        NativeLanguage::English => format!("Update to {version}…"),
        NativeLanguage::Chinese => format!("更新到 {version}…"),
    }
}

/// macOS 上应用没在“应用程序”文件夹里、被系统挪到只读路径（App Translocation），或者直接
/// 从磁盘映像里运行时，更新换不了自己。
pub(crate) fn running_from_read_only_location(executable: &Path) -> bool {
    let path = executable.to_string_lossy();
    path.contains("/AppTranslocation/") || path.starts_with("/Volumes/")
}

pub(crate) fn app_translocated() -> bool {
    cfg!(target_os = "macos")
        && std::env::current_exe()
            .is_ok_and(|executable| running_from_read_only_location(&executable))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_hours(1);

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn 没查过就该查_满了间隔才再查_时钟往回跳也查() {
        assert!(due(None, at(1_000), AUTOMATIC_INTERVAL));
        assert!(!due(
            Some(at(1_000)),
            at(1_000 + 3 * 3_600),
            AUTOMATIC_INTERVAL
        ));
        assert!(due(
            Some(at(1_000)),
            at(1_000 + 4 * 3_600),
            AUTOMATIC_INTERVAL
        ));
        // 睡了一夜：墙上时间早就过了 4 小时。
        assert!(due(
            Some(at(1_000)),
            at(1_000 + 9 * 3_600),
            AUTOMATIC_INTERVAL
        ));
        assert!(due(Some(at(10_000)), at(5_000), AUTOMATIC_INTERVAL));
        assert!(!due(
            Some(at(1_000)),
            at(1_000 + 1_800),
            WINDOW_RECHECK_AFTER
        ));
        assert!(due(Some(at(1_000)), at(1_000 + 3_600), HOUR));
    }

    fn check(available: bool, target: &str) -> ReleaseUpdateCheck {
        ReleaseUpdateCheck::for_tests(available, "0.1.0-alpha.67", target)
    }

    #[test]
    fn 没查成时留着上次查成的结果_查成了就清掉原因() {
        let found =
            ReleaseUpdateStatus::after(None, "testing", &Ok(check(true, "0.1.0-alpha.68")), 10);
        assert_eq!(found.available_version(), Some("0.1.0-alpha.68"));
        assert_eq!(found.failure, None);

        let offline = ReleaseUpdateStatus::after(
            Some(&found),
            "testing",
            &Err(ReleaseUpdateFailure::for_tests(
                "desktop.update.manifest_network",
                true,
            )),
            20,
        );
        assert_eq!(offline.checked_at_unix_ms, 20);
        assert_eq!(offline.available_version(), Some("0.1.0-alpha.68"));
        assert_eq!(
            offline.failure,
            Some(ReleaseUpdateStatusFailure {
                code: "desktop.update.manifest_network",
                retryable: true
            })
        );

        let current = ReleaseUpdateStatus::after(
            Some(&offline),
            "testing",
            &Ok(check(false, "0.1.0-alpha.67")),
            30,
        );
        assert_eq!(current.available_version(), None);
        assert_eq!(current.failure, None);

        let first_failure = ReleaseUpdateStatus::after(
            None,
            "stable",
            &Err(ReleaseUpdateFailure::for_tests(
                "desktop.update.manifest_expired",
                false,
            )),
            40,
        );
        assert_eq!(first_failure.check, None);
        assert_eq!(first_failure.channel, "stable");
    }

    #[test]
    fn 每个版本只提醒一次_重启以后也记得() {
        let directory = tempfile::tempdir().expect("临时目录");
        let marker = directory.path().join("desktop").join("update-notified");
        let notice = UpdateNotice::new(marker.clone());
        assert!(notice.claim("0.1.0-alpha.68"));
        assert!(!notice.claim("0.1.0-alpha.68"));
        assert!(!UpdateNotice::new(marker.clone()).claim("0.1.0-alpha.68"));
        assert!(UpdateNotice::new(marker).claim("0.1.0-alpha.69"));
    }

    #[test]
    fn 通知和托盘的说法跟着语言() {
        let (title, body) = available_notification(NativeLanguage::Chinese, "0.1.0-alpha.68");
        assert_eq!(title, "Agent Room 有新版本");
        assert_eq!(
            body,
            "0.1.0-alpha.68 可以安装了。打开 Agent Room，点“更新并重启”。"
        );
        let (title, body) = available_notification(NativeLanguage::English, "0.1.0-alpha.68");
        assert_eq!(title, "Agent Room update available");
        assert!(body.starts_with("0.1.0-alpha.68 is ready to install."));
        assert_eq!(
            tray_update_label(NativeLanguage::Chinese, "0.1.0-alpha.68"),
            "更新到 0.1.0-alpha.68…"
        );
        assert_eq!(
            tray_update_label(NativeLanguage::English, "0.1.0-alpha.68"),
            "Update to 0.1.0-alpha.68…"
        );
    }

    #[test]
    fn 只读位置_挪到只读路径或者从磁盘映像里运行() {
        assert!(running_from_read_only_location(Path::new(
            "/private/var/folders/x/T/AppTranslocation/1A2B/d/Agent Room.app/Contents/MacOS/agent-room-desktop"
        )));
        assert!(running_from_read_only_location(Path::new(
            "/Volumes/Agent Room/Agent Room.app/Contents/MacOS/agent-room-desktop"
        )));
        assert!(!running_from_read_only_location(Path::new(
            "/Applications/Agent Room.app/Contents/MacOS/agent-room-desktop"
        )));
        assert!(!running_from_read_only_location(Path::new(
            r"C:\Users\someone\AppData\Local\Agent Room\agent-room-desktop.exe"
        )));
    }
}
