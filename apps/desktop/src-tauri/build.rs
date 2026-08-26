const COMMANDS: &[&str] = &[
    "desktop_runtime_snapshot",
    "desktop_retry_bridge",
    "desktop_set_autostart",
    "desktop_open_authorization",
    "desktop_check_update",
    "desktop_install_update",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("Tauri 桌面构建配置必须有效");
}
