use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::{
    bridge_supervisor::{BridgeRuntimeView, BridgeSupervisor, SupervisorFailure},
    deep_link::{DeepLinkInbox, DeepLinkTarget},
    release_updates::{
        ReleaseUpdateCheck, ReleaseUpdateFailure, ReleaseUpdateRuntime, parse_channel,
    },
};

#[derive(Clone)]
pub(crate) struct DesktopRuntime {
    pub(crate) bridge: BridgeSupervisor,
    pub(crate) updates: ReleaseUpdateRuntime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopRuntimeSnapshot {
    bridge: BridgeRuntimeView,
    autostart_enabled: bool,
    platform: &'static str,
    deep_link: Option<DeepLinkTarget>,
    updates_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopCommandFailure {
    code: String,
    retryable: bool,
}

impl DesktopCommandFailure {
    fn new(code: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            retryable,
        }
    }
}

impl From<SupervisorFailure> for DesktopCommandFailure {
    fn from(failure: SupervisorFailure) -> Self {
        Self::new(failure.code, failure.retryable)
    }
}

impl From<ReleaseUpdateFailure> for DesktopCommandFailure {
    fn from(failure: ReleaseUpdateFailure) -> Self {
        Self::new(failure.code(), failure.retryable())
    }
}

#[tauri::command]
// Tauri 命令宏按值提取 AppHandle 与 State；改成借用会破坏命令参数解析。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_runtime_snapshot(
    app: AppHandle,
    runtime: State<'_, DesktopRuntime>,
    deep_links: State<'_, DeepLinkInbox>,
) -> Result<DesktopRuntimeSnapshot, DesktopCommandFailure> {
    let autostart_enabled = app
        .autolaunch()
        .is_enabled()
        .map_err(|_| DesktopCommandFailure::new("desktop.autostart.read_failed", true))?;
    Ok(DesktopRuntimeSnapshot {
        bridge: runtime.bridge.snapshot(),
        autostart_enabled,
        platform: current_platform(),
        deep_link: deep_links.latest(),
        updates_configured: runtime.updates.configured(),
    })
}

#[tauri::command]
// Tauri 命令宏按值提取 State 并反序列化渠道；克隆运行时后才跨越 await。
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn desktop_check_update(
    runtime: State<'_, DesktopRuntime>,
    channel: String,
) -> Result<ReleaseUpdateCheck, DesktopCommandFailure> {
    let updates = runtime.updates.clone();
    Ok(updates.check(parse_channel(&channel)?).await?)
}

#[tauri::command]
// Tauri 命令宏按值提取 State 并反序列化参数；克隆运行时后才跨越 await。
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn desktop_install_update(
    runtime: State<'_, DesktopRuntime>,
    channel: String,
    expected_sequence: u64,
) -> Result<(), DesktopCommandFailure> {
    let updates = runtime.updates.clone();
    updates
        .install(parse_channel(&channel)?, expected_sequence)
        .await?;
    Ok(())
}

#[tauri::command]
// Tauri 命令宏按值提取 State；这里不是普通函数调用边界。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_retry_bridge(
    runtime: State<'_, DesktopRuntime>,
) -> Result<BridgeRuntimeView, DesktopCommandFailure> {
    runtime.bridge.retry()?;
    Ok(runtime.bridge.snapshot())
}

#[tauri::command]
// Tauri 命令宏按值提取 AppHandle；这里不是普通函数调用边界。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_set_autostart(
    app: AppHandle,
    enabled: bool,
) -> Result<bool, DesktopCommandFailure> {
    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|_| DesktopCommandFailure::new("desktop.autostart.enable_failed", true))?;
    } else {
        manager
            .disable()
            .map_err(|_| DesktopCommandFailure::new("desktop.autostart.disable_failed", true))?;
    }
    let observed = manager
        .is_enabled()
        .map_err(|_| DesktopCommandFailure::new("desktop.autostart.read_failed", true))?;
    if observed != enabled {
        return Err(DesktopCommandFailure::new(
            "desktop.autostart.state_mismatch",
            true,
        ));
    }
    Ok(observed)
}

#[tauri::command]
// Tauri 命令宏按值反序列化参数并提取运行时状态。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_open_authorization(
    app: AppHandle,
    runtime: State<'_, DesktopRuntime>,
    prompt_id: String,
) -> Result<(), DesktopCommandFailure> {
    let url = runtime.bridge.authorization_url(&prompt_id)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| DesktopCommandFailure::new("desktop.authorization.open_failed", true))
}

const fn current_platform() -> &'static str {
    #[cfg(target_os = "windows")]
    return "windows";
    #[cfg(target_os = "macos")]
    return "macos";
    #[cfg(target_os = "linux")]
    return "linux";
    #[allow(unreachable_code)]
    "unknown"
}
