mod bridge_lifecycle;
mod bridge_supervisor;
#[cfg(test)]
mod capability_tests;
mod commands;
mod deep_link;
mod desktop_config;
mod webview_migration;

use commands::{
    DesktopRuntime, desktop_open_authorization, desktop_retry_bridge, desktop_runtime_snapshot,
    desktop_set_autostart,
};
use deep_link::{DeepLinkInbox, deliver_deep_links};
use desktop_config::DesktopBridgeConfig;
use tauri::{
    Manager as _, RunEvent,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_deep_link::DeepLinkExt as _;

/// 启动 Agent Room 桌面壳并接管 Bridge 生命周期。
///
/// # Panics
///
/// 当 Tauri 上下文、窗口或插件无法构建时会终止启动。此时继续运行会留下一个
/// 没有受监管 Bridge 的残缺桌面进程，因此必须显式失败。
pub fn run() {
    let mut builder = tauri::Builder::default();
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                show_main_window(app);
            },
        ));
    }
    let app = builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(DeepLinkInbox::default())
        .invoke_handler(tauri::generate_handler![
            desktop_runtime_snapshot,
            desktop_retry_bridge,
            desktop_set_autostart,
            desktop_open_authorization,
        ])
        .setup(|app| {
            webview_migration::retire_legacy_service_worker(app)?;
            let config = DesktopBridgeConfig::from_environment()
                .map_err(|failure| format!("桌面 Bridge 配置失败 [{}]", failure.code()))?;
            let bridge = bridge_supervisor::BridgeSupervisor::start(app.handle().clone(), config);
            app.manage(DesktopRuntime { bridge });
            setup_tray(app)?;
            setup_deep_links(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Agent Room 桌面壳必须能够构建");

    app.run(|app, event| match event {
        RunEvent::Resumed => app.state::<DesktopRuntime>().bridge.resume(),
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            app.state::<DesktopRuntime>().bridge.shutdown_now();
        }
        _ => {}
    });
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Agent Room", true, None::<&str>)?;
    let retry = MenuItem::with_id(app, "retry", "Retry Bridge", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &retry, &quit])?;
    let mut tray = TrayIconBuilder::with_id("agent-room")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Agent Room");
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id().as_ref() {
        "open" => show_main_window(app),
        "retry" => {
            let _ = app.state::<DesktopRuntime>().bridge.retry();
            show_main_window(app);
        }
        "quit" => {
            app.state::<DesktopRuntime>().bridge.shutdown_now();
            app.exit(0);
        }
        _ => {}
    })
    .build(app)?;
    Ok(())
}

fn setup_deep_links(app: &mut tauri::App) -> Result<(), tauri_plugin_deep_link::Error> {
    #[cfg(debug_assertions)]
    #[cfg(any(windows, target_os = "linux"))]
    app.deep_link().register_all()?;

    if let Some(urls) = app.deep_link().get_current()? {
        deliver_deep_links(app.handle(), urls);
    }
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        deliver_deep_links(&handle, event.urls().iter().cloned());
    });
    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
