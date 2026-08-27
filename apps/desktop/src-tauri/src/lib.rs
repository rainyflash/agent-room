mod bridge_lifecycle;
mod bridge_supervisor;
#[cfg(test)]
mod capability_tests;
mod commands;
mod deep_link;
mod desktop_config;
mod release_update_config;
mod release_update_state;
mod release_updates;
mod runtime_target;
mod webview_migration;

use agent_room_host_adapters::{HostConfigurator, HostContext};
use commands::{
    DesktopRuntime, desktop_apply_agent_host, desktop_check_update,
    desktop_configure_agent_runtime, desktop_detect_agent_hosts, desktop_install_update,
    desktop_open_authorization, desktop_plan_agent_host, desktop_remove_agent_host,
    desktop_retry_bridge, desktop_runtime_snapshot, desktop_set_autostart,
};
use deep_link::{DeepLinkInbox, deliver_deep_links};
use desktop_config::DesktopBridgeConfig;
use release_update_config::ReleaseUpdateConfig;
use release_updates::ReleaseUpdateRuntime;
use runtime_target::RuntimeTargetStore;
use std::{path::PathBuf, sync::Arc};
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
    let update_config = ReleaseUpdateConfig::from_build().unwrap_or_else(|failure| {
        panic!("桌面签名更新配置失败 [{}]", failure.code());
    });
    let mut builder = tauri::Builder::default();
    if let Some(config) = &update_config {
        builder = builder.plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(config.tauri_public_key().to_owned())
                .build(),
        );
    }
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
            desktop_check_update,
            desktop_install_update,
            desktop_detect_agent_hosts,
            desktop_plan_agent_host,
            desktop_apply_agent_host,
            desktop_remove_agent_host,
            desktop_configure_agent_runtime,
        ])
        .setup(move |app| {
            webview_migration::retire_legacy_service_worker(app)?;
            let mut config = DesktopBridgeConfig::from_environment()
                .map_err(|failure| format!("桌面 Bridge 配置失败 [{}]", failure.code()))?;
            let targets = Arc::new(
                RuntimeTargetStore::open(&config.data_root())
                    .map_err(|failure| format!("桌面 Agent 目标读取失败 [{}]", failure.code()))?,
            );
            if let Some(target) = targets
                .current()
                .map_err(|failure| format!("桌面 Agent 目标读取失败 [{}]", failure.code()))?
            {
                config = config.with_agent_target(&target);
            }
            let bridge = bridge_supervisor::BridgeSupervisor::start(app.handle().clone(), config);
            let updates = ReleaseUpdateRuntime::new(app.handle().clone(), update_config.clone())
                .map_err(|failure| format!("桌面更新状态初始化失败 [{}]", failure.code()))?;
            let mcp_executable = installed_mcp_executable()?;
            let host_context = HostContext::from_environment(mcp_executable)
                .map_err(|failure| format!("宿主配置器初始化失败 [{}]", failure.code()))?;
            let hosts = Arc::new(HostConfigurator::system(host_context));
            app.manage(DesktopRuntime {
                bridge,
                updates,
                hosts,
                targets,
            });
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

fn installed_mcp_executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|_| "无法定位桌面程序目录".to_owned())?;
    let directory = executable
        .parent()
        .ok_or_else(|| "桌面程序目录无效".to_owned())?;
    let filename = if cfg!(windows) {
        "agent-room-mcp.exe"
    } else {
        "agent-room-mcp"
    };
    let path = directory.join(filename);
    if path.is_file() {
        Ok(path)
    } else {
        Err("安装包缺少 agent-room-mcp".to_owned())
    }
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
