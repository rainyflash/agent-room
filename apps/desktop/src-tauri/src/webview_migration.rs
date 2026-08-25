use tauri::Manager as _;

const RETIRE_LEGACY_SERVICE_WORKER: &str = r#"
void (async () => {
  if (!("serviceWorker" in navigator)) return;
  const registrations = await navigator.serviceWorker.getRegistrations();
  if (registrations.length === 0) return;
  await Promise.all(registrations.map((registration) => registration.unregister()));
  if ("caches" in window) {
    const names = await caches.keys();
    await Promise.all(names.map((name) => caches.delete(name)));
  }
  window.location.reload();
})().catch(() => {
  document.documentElement.dataset.desktopMigrationFailure =
    "desktop.service_worker_retirement_failed";
});
"#;

/// 注销早期桌面测试版错误安装的 PWA Service Worker。
///
/// 浏览器构建仍可使用 PWA；桌面壳必须始终加载随二进制签发的同版本资源，不能让
/// Service Worker 用旧缓存制造 Rust/TypeScript 协议错配。
pub(crate) fn retire_legacy_service_worker(app: &tauri::App) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    window.eval(RETIRE_LEGACY_SERVICE_WORKER)
}

#[cfg(test)]
mod tests {
    use super::RETIRE_LEGACY_SERVICE_WORKER;

    #[test]
    fn 迁移脚本只清理_service_worker_与_cache_storage() {
        assert!(RETIRE_LEGACY_SERVICE_WORKER.contains("registration.unregister()"));
        assert!(RETIRE_LEGACY_SERVICE_WORKER.contains("caches.delete(name)"));
        assert!(!RETIRE_LEGACY_SERVICE_WORKER.contains("localStorage"));
        assert!(!RETIRE_LEGACY_SERVICE_WORKER.contains("clearAllBrowsingData"));
    }
}
