use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    Manager,
    menu::{IsMenuItem, Menu, MenuItem},
};

use crate::{commands::DesktopRuntime, release_update_watch::tray_update_label};

/// 托盘图标的 ID。
pub(crate) const TRAY_ID: &str = "agent-room";
/// 有新版本时托盘菜单里“更新到 X…”那一项的 ID。
pub(crate) const UPDATE_MENU_ID: &str = "update";

#[derive(Clone, Copy, Deserialize)]
pub(crate) enum NativeLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
}

#[derive(Default)]
pub(crate) struct NativeLanguageState(AtomicBool);

pub(crate) fn language(app: &tauri::AppHandle) -> NativeLanguage {
    if app.state::<NativeLanguageState>().0.load(Ordering::Relaxed) {
        NativeLanguage::Chinese
    } else {
        NativeLanguage::English
    }
}

/// 托盘菜单。查到新版本时在“打开”下面多一项“更新到 X…”。
pub(crate) fn tray_menu(
    app: &tauri::AppHandle,
    language: NativeLanguage,
    update: Option<&str>,
) -> tauri::Result<Menu<tauri::Wry>> {
    let labels = match language {
        NativeLanguage::English => ["Open Agent Room", "Reconnect", "Quit"],
        NativeLanguage::Chinese => ["打开 Agent Room", "重新连接", "退出"],
    };
    let open = MenuItem::with_id(app, "open", labels[0], true, None::<&str>)?;
    let update = update
        .map(|version| {
            MenuItem::with_id(
                app,
                UPDATE_MENU_ID,
                tray_update_label(language, version),
                true,
                None::<&str>,
            )
        })
        .transpose()?;
    let retry = MenuItem::with_id(app, "retry", labels[1], true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels[2], true, None::<&str>)?;
    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = vec![&open];
    if let Some(update) = &update {
        items.push(update);
    }
    items.push(&retry);
    items.push(&quit);
    Menu::with_items(app, &items)
}

/// 可装的新版本变了：用当前语言重建托盘菜单。重建不成只记日志，窗口里的提示照样在。
pub(crate) fn refresh_tray_menu(app: &tauri::AppHandle, update: Option<&str>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let rebuilt = tray_menu(app, language(app), update).and_then(|menu| tray.set_menu(Some(menu)));
    if let Err(error) = rebuilt {
        tracing::warn!(%error, "托盘菜单没能按更新状态重建");
    }
}

pub(crate) fn stopped_message(language: NativeLanguage) -> (&'static str, &'static str) {
    match language {
        NativeLanguage::English => (
            "Agent Room connection stopped",
            "The connection could not be restored. Open Agent Room to see the reason and reconnect.",
        ),
        NativeLanguage::Chinese => (
            "Agent Room 连接已停止",
            "暂时无法恢复连接。请打开 Agent Room 查看原因，再点“重新连接”。",
        ),
    }
}

#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri owns command arguments at the IPC boundary"
)]
pub(crate) fn desktop_set_language(
    app: tauri::AppHandle,
    language: NativeLanguage,
) -> Result<(), String> {
    // 先记下语言，这之后按更新状态重建菜单的也用新语言。
    app.state::<NativeLanguageState>().0.store(
        matches!(language, NativeLanguage::Chinese),
        Ordering::Relaxed,
    );
    // 用当时的更新状态重建，别把“更新到 X…”丢了。
    let update = app
        .try_state::<DesktopRuntime>()
        .and_then(|runtime| runtime.updates.available_version());
    let menu = tray_menu(&app, language, update.as_deref()).map_err(|error| error.to_string())?;
    let tray = app.tray_by_id(TRAY_ID).ok_or("desktop.tray.unavailable")?;
    tray.set_menu(Some(menu))
        .map_err(|error| error.to_string())?;
    Ok(())
}
