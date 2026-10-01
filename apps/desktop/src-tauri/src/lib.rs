mod agent_runtime;
mod authentication_values;
mod bridge_lifecycle;
mod bridge_supervisor;
#[cfg(test)]
mod capability_tests;
mod commands;
#[cfg(all(test, windows))]
mod credential_store_test_support;
mod deep_link;
#[cfg(test)]
mod desktop_command_surface;
mod desktop_config;
mod human_session;
mod installer_acceptance;
mod logging;
use logging::desktop_open_logs;
mod loopback_callback;
mod matrix_credentials;
mod matrix_session;
mod native_language;
mod notifications;
use native_language::desktop_set_language;
use notifications::desktop_notify;
mod receiver_runtime;
mod release_update_config;
mod release_update_state;
mod release_updates;
mod runtime_target;
mod server_move;
mod tray_hint;
use receiver_runtime::{
    ReceiverRuntime, desktop_receiver_action, desktop_receiver_configure, desktop_receiver_list,
};
mod webview_migration;

use agent_room_host_adapters::{HostConfigurator, HostContext};
use commands::{
    DesktopRuntime, desktop_agent_recovery, desktop_agent_recovery_sessions,
    desktop_begin_human_authentication, desktop_begin_matrix_authentication,
    desktop_bootstrap_default_agent, desktop_check_update, desktop_clear_human_session,
    desktop_clear_matrix_session, desktop_configure_agent_runtime,
    desktop_host_session_diagnostics, desktop_install_update, desktop_load_matrix_session,
    desktop_offer_invitation, desktop_open_authorization, desktop_reauthorize_bridge,
    desktop_restore_human_session, desktop_retry_bridge, desktop_runtime_snapshot,
    desktop_save_matrix_session, desktop_set_autostart, desktop_withdraw_invitation,
};
use deep_link::{DeepLinkInbox, deliver_deep_links};
use desktop_config::DesktopBridgeConfig;
use human_session::HumanSessionRuntime;
use matrix_credentials::MatrixCredentialRuntime;
use matrix_session::MatrixSessionRuntime;
use release_update_config::ReleaseUpdateConfig;
use release_updates::ReleaseUpdateRuntime;
use runtime_target::RuntimeTargetStore;
use std::{path::PathBuf, process::ExitCode, sync::Arc};
use tauri::{Manager as _, RunEvent, tray::TrayIconBuilder};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_deep_link::DeepLinkExt as _;

/// 按进程参数选择交互桌面或无 `WebView` 的安装器验收入口。
///
/// 安装器验收仍使用正式桌面的 Bridge 配置并持有子进程生命周期，但不会在
/// GitHub Windows runner 的非交互会话中创建 `WebView` 窗口。
pub fn run_entrypoint() -> ExitCode {
    match installer_acceptance::launch_mode(std::env::args_os().skip(1)) {
        installer_acceptance::DesktopLaunchMode::Interactive => {
            let update_config = match ReleaseUpdateConfig::from_build() {
                Ok(config) => config,
                Err(failure) => {
                    eprintln!("Agent Room 启动失败 [{}]", failure.code());
                    return ExitCode::FAILURE;
                }
            };
            run(update_config);
            ExitCode::SUCCESS
        }
        installer_acceptance::DesktopLaunchMode::InstallerAcceptance => installer_acceptance::run(),
        installer_acceptance::DesktopLaunchMode::InstallerVersion => {
            installer_acceptance::print_version()
        }
    }
}

/// 启动 Agent Room 桌面壳并接管 Bridge 生命周期。
///
/// # Panics
///
/// 当 Tauri 上下文、窗口或插件无法构建时会终止启动。此时继续运行会留下一个
/// 没有受监管 Bridge 的残缺桌面进程，因此必须显式失败。
fn run(update_config: Option<ReleaseUpdateConfig>) {
    // Agent 连不上 Bridge 时会带这个参数在后台拉起桌面端：只跑 Bridge 与托盘，不弹主窗口。
    let background = launched_in_background(std::env::args().skip(1));
    let mut builder = configure_updater(tauri::Builder::default(), update_config.as_ref());
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(
            |app, arguments, _cwd| {
                // 应用已在运行时点击房间链接：链接随第二个实例的参数到来，不能只把窗口拉到前面。
                let urls = deep_link::deep_links_in_arguments(arguments.iter().map(String::as_str));
                if !urls.is_empty() {
                    deliver_deep_links(app, urls);
                } else if !launched_in_background(arguments.iter().cloned()) {
                    show_main_window(app);
                }
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
        .manage(native_language::NativeLanguageState::default())
        .invoke_handler(tauri::generate_handler![
            desktop_begin_human_authentication,
            desktop_begin_matrix_authentication,
            desktop_load_matrix_session,
            desktop_save_matrix_session,
            desktop_clear_matrix_session,
            desktop_clear_human_session,
            desktop_restore_human_session,
            desktop_runtime_snapshot,
            desktop_retry_bridge,
            desktop_reauthorize_bridge,
            desktop_set_autostart,
            desktop_set_language,
            desktop_open_authorization,
            desktop_open_logs,
            desktop_notify,
            desktop_check_update,
            desktop_install_update,
            desktop_bootstrap_default_agent,
            desktop_configure_agent_runtime,
            desktop_agent_recovery_sessions,
            desktop_host_session_diagnostics,
            desktop_offer_invitation,
            desktop_withdraw_invitation,
            desktop_agent_recovery,
            desktop_receiver_list,
            desktop_receiver_configure,
            desktop_receiver_action,
        ])
        .setup(move |app| {
            let result = setup_runtime(app, update_config.clone());
            if let Err(error) = &result {
                tracing::error!(%error, "桌面端初始化失败");
            }
            // 主窗口建好时是隐藏的：普通启动在这里显示，后台启动留在托盘里。
            if !background {
                show_main_window(app.handle());
            }
            result
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
                tray_hint::notify_first_hide(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
        .expect("Agent Room 桌面壳必须能够构建");

    app.run(on_run_event);
}

fn on_run_event(app: &tauri::AppHandle, event: RunEvent) {
    match event {
        RunEvent::Resumed => app.state::<DesktopRuntime>().bridge.resume(),
        RunEvent::ExitRequested { api, .. } => {
            let runtime = app.state::<DesktopRuntime>().inner().clone();
            if runtime.receivers.begin_shutdown() {
                api.prevent_exit();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = runtime.receivers.shutdown().await {
                        tracing::warn!(error_code = error.code, "接收进程关闭失败");
                    }
                    // 请 Bridge 有序退出：各人物发出离开状态，进行中的同步收尾，不在加密存储写到一半时被结束。
                    runtime.bridge.shutdown().await;
                    app.exit(0);
                });
            }
        }
        RunEvent::Exit => app.state::<DesktopRuntime>().bridge.shutdown_now(),
        _ => {}
    }
}

fn setup_runtime(
    app: &mut tauri::App,
    update_config: Option<ReleaseUpdateConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    match_window_to_system_theme(app);
    webview_migration::retire_legacy_service_worker(app)?;
    let mut config = DesktopBridgeConfig::from_environment()
        .map_err(|failure| format!("桌面 Bridge 配置失败 [{}]", failure.code()))?;
    let logs = logging::LogLocation::new(&config.data_root());
    let log_path = logs.install();
    app.manage(logs);
    tracing::info!(
        version = app.package_info().version.to_string(),
        log_file = log_path.as_deref().map(|path| path.display().to_string()),
        "Agent Room 桌面端启动"
    );
    server_move::retire_previous_server(&config);
    setup_user_sessions(app, &config)?;
    app.manage(tray_hint::TrayHint::new(&config.data_root()));
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
    let bridge = bridge_supervisor::BridgeSupervisor::start(app.handle().clone(), config.clone());
    let updates = ReleaseUpdateRuntime::new(app.handle().clone(), update_config)
        .map_err(|failure| format!("桌面更新状态初始化失败 [{}]", failure.code()))?;
    let mcp_executable = installed_mcp_executable()?;
    let receivers = ReceiverRuntime::new(config, mcp_executable.clone());
    let restored_receivers = receivers.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = restored_receivers.restore().await {
            tracing::warn!(error_code = error.code, "接收进程恢复失败");
        }
    });
    let host_context = HostContext::from_environment(mcp_executable)
        .map_err(|failure| format!("本机宿主信息初始化失败 [{}]", failure.code()))?;
    let hosts = Arc::new(HostConfigurator::system(host_context));
    app.manage(DesktopRuntime {
        bridge,
        receivers,
        updates,
        hosts,
        targets,
    });
    setup_tray(app)?;
    setup_deep_links(app)?;
    Ok(())
}

fn setup_user_sessions(app: &tauri::App, config: &DesktopBridgeConfig) -> Result<(), String> {
    let human_sessions = HumanSessionRuntime::system(config)
        .map_err(|failure| format!("桌面人类会话初始化失败 [{}]", failure.code()))?;
    // 由前端启动时的 restore 命令恢复，保证请求顺序并把存储故障呈现为可恢复界面。
    app.manage(human_sessions);
    app.manage(MatrixSessionRuntime::system(config));
    app.manage(MatrixCredentialRuntime::system(config));
    Ok(())
}

fn configure_updater(
    mut builder: tauri::Builder<tauri::Wry>,
    update_config: Option<&ReleaseUpdateConfig>,
) -> tauri::Builder<tauri::Wry> {
    if let Some(config) = &update_config {
        builder = builder.plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(config.tauri_public_key().to_owned())
                .build(),
        );
    }
    builder
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

/// 系统是暗色时窗口底色也换成暗色：页面样式跟随系统，底色不换的话加载前会先闪一下白。
fn match_window_to_system_theme(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main")
        && matches!(window.theme(), Ok(tauri::Theme::Dark))
    {
        let _ = window.set_background_color(Some(tauri::window::Color(19, 18, 23, 255)));
    }
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let menu = native_language::tray_menu(app.handle(), native_language::language(app.handle()))?;
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

/// 带 `--background` 启动：由 Agent 在连不上 Bridge 时拉起，不弹主窗口。
fn launched_in_background(arguments: impl IntoIterator<Item = String>) -> bool {
    arguments
        .into_iter()
        .any(|argument| argument == agent_room_agent_client::DESKTOP_BACKGROUND_ARGUMENT)
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod background_launch_tests {
    use super::launched_in_background;

    #[test]
    fn 只有带_background_参数的启动才留在托盘里() {
        let arguments = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(launched_in_background(arguments(&["--background"])));
        assert!(launched_in_background(arguments(&[
            "--autostart",
            "--background"
        ])));
        assert!(!launched_in_background(arguments(&[])));
        assert!(!launched_in_background(arguments(&["--autostart"])));
        assert!(!launched_in_background(arguments(&[
            "agent-room://room/abc"
        ])));
    }
}
