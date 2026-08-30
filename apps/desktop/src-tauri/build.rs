const COMMANDS: &[&str] = &[
    "desktop_begin_human_authentication",
    "desktop_clear_human_session",
    "desktop_runtime_snapshot",
    "desktop_retry_bridge",
    "desktop_set_autostart",
    "desktop_open_authorization",
    "desktop_check_update",
    "desktop_install_update",
    "desktop_detect_agent_hosts",
    "desktop_plan_agent_host",
    "desktop_apply_agent_host",
    "desktop_remove_agent_host",
    "desktop_bootstrap_default_agent",
    "desktop_configure_agent_runtime",
    "desktop_lobby_snapshot",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("Tauri 桌面构建配置必须有效");
}
