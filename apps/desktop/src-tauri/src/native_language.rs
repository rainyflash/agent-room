use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
};

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

pub(crate) fn tray_menu(
    app: &tauri::AppHandle,
    language: NativeLanguage,
) -> tauri::Result<Menu<tauri::Wry>> {
    let labels = match language {
        NativeLanguage::English => ["Open Agent Room", "Reconnect", "Quit"],
        NativeLanguage::Chinese => ["打开 Agent Room", "重新连接", "退出"],
    };
    let open = MenuItem::with_id(app, "open", labels[0], true, None::<&str>)?;
    let retry = MenuItem::with_id(app, "retry", labels[1], true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels[2], true, None::<&str>)?;
    Menu::with_items(app, &[&open, &retry, &quit])
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
    let menu = tray_menu(&app, language).map_err(|error| error.to_string())?;
    let tray = app
        .tray_by_id("agent-room")
        .ok_or("desktop.tray.unavailable")?;
    tray.set_menu(Some(menu))
        .map_err(|error| error.to_string())?;
    app.state::<NativeLanguageState>().0.store(
        matches!(language, NativeLanguage::Chinese),
        Ordering::Relaxed,
    );
    Ok(())
}
