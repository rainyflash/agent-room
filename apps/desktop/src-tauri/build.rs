include!("src/desktop_command_surface.rs");

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(DESKTOP_COMMANDS));
    tauri_build::try_build(attributes).expect("Tauri 桌面构建配置必须有效");
}
