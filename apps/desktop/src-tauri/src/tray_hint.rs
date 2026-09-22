//! 关窗只是隐藏到托盘。第一次这样做时提醒一句，之后不再打扰；托盘提示文字随连接状态更新。

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager as _};
use tauri_plugin_notification::NotificationExt as _;

use crate::{
    bridge_lifecycle::BridgePhase,
    native_language::{self, NativeLanguage},
};

const MARKER_FILENAME: &str = "tray-hint-shown";
const TRAY_ID: &str = "agent-room";

/// 记住「已经提醒过窗口只是隐藏到托盘」。
pub(crate) struct TrayHint {
    marker: PathBuf,
}

impl TrayHint {
    pub(crate) fn new(data_root: &Path) -> Self {
        Self {
            marker: data_root.join(MARKER_FILENAME),
        }
    }

    /// 只在第一次隐藏时返回 `true`，同时记下已提醒。记不住的话最多再提醒一次，不算失败。
    pub(crate) fn claim_first_hide(&self) -> bool {
        if self.marker.is_file() {
            return false;
        }
        if let Some(parent) = self.marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.marker, b"1\n");
        true
    }
}

/// 窗口被关进托盘后调用：第一次弹一条系统通知说明应用仍在运行。
pub(crate) fn notify_first_hide(app: &AppHandle) {
    let Some(hint) = app.try_state::<TrayHint>() else {
        return;
    };
    if !hint.claim_first_hide() {
        return;
    }
    let (title, body) = hidden_message(native_language::language(app));
    let _ = app.notification().builder().title(title).body(body).show();
}

/// 把连接状态写进托盘图标的悬停提示，窗口藏起来时也能一眼看到。
pub(crate) fn update_tooltip(app: &AppHandle, phase: BridgePhase) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip(native_language::language(app), phase)));
    }
}

pub(crate) fn hidden_message(language: NativeLanguage) -> (&'static str, &'static str) {
    match language {
        NativeLanguage::English => (
            "Agent Room keeps running in the background",
            "Agents keep replying with the window closed. Use the tray icon to open Agent Room again or to quit.",
        ),
        NativeLanguage::Chinese => (
            "Agent Room 仍在后台运行",
            "窗口关了 Agent 照常回复。要再打开或退出，请用托盘图标。",
        ),
    }
}

pub(crate) fn tooltip(language: NativeLanguage, phase: BridgePhase) -> &'static str {
    match (language, phase) {
        (NativeLanguage::English, BridgePhase::Ready | BridgePhase::Authorized) => {
            "Agent Room · connected"
        }
        (
            NativeLanguage::English,
            BridgePhase::Discovering
            | BridgePhase::Starting
            | BridgePhase::Reconnecting
            | BridgePhase::RetryScheduled,
        ) => "Agent Room · connecting…",
        (NativeLanguage::English, BridgePhase::AuthorizationRequired) => {
            "Agent Room · authorization needed"
        }
        (NativeLanguage::English, BridgePhase::Halted | BridgePhase::Stopped) => {
            "Agent Room · connection stopped"
        }
        (NativeLanguage::Chinese, BridgePhase::Ready | BridgePhase::Authorized) => {
            "Agent Room · 已连接"
        }
        (
            NativeLanguage::Chinese,
            BridgePhase::Discovering
            | BridgePhase::Starting
            | BridgePhase::Reconnecting
            | BridgePhase::RetryScheduled,
        ) => "Agent Room · 连接中…",
        (NativeLanguage::Chinese, BridgePhase::AuthorizationRequired) => "Agent Room · 需要授权",
        (NativeLanguage::Chinese, BridgePhase::Halted | BridgePhase::Stopped) => {
            "Agent Room · 连接已停止"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BridgePhase, NativeLanguage, TrayHint, tooltip};

    #[test]
    fn 只提醒第一次隐藏() {
        let directory = tempfile::tempdir().expect("临时目录");
        let hint = TrayHint::new(&directory.path().join("desktop"));
        assert!(hint.claim_first_hide());
        assert!(!hint.claim_first_hide());
        // 新的实例读同一目录，重启后也不再提醒。
        assert!(!TrayHint::new(&directory.path().join("desktop")).claim_first_hide());
    }

    #[test]
    fn 托盘提示随状态和语言变化() {
        assert_eq!(
            tooltip(NativeLanguage::Chinese, BridgePhase::Ready),
            "Agent Room · 已连接"
        );
        assert_eq!(
            tooltip(NativeLanguage::English, BridgePhase::AuthorizationRequired),
            "Agent Room · authorization needed"
        );
        assert_eq!(
            tooltip(NativeLanguage::Chinese, BridgePhase::Halted),
            "Agent Room · 连接已停止"
        );
    }
}
