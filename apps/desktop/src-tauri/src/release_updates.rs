use std::{
    fmt::Write as _,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};

use agent_room_release_manifest::{
    ArtifactKind, ReleaseArtifact, ReleaseChannel, ReleaseInspection, ReleaseManifestError,
    SignedReleaseManifest, VerifiedRelease, inspect_release,
};
use futures_util::StreamExt as _;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Emitter as _, Manager as _};
use tauri_plugin_notification::NotificationExt as _;
use tauri_plugin_updater::{Update, UpdaterExt as _};
use url::Url;

use crate::{
    native_language,
    release_update_config::ReleaseUpdateConfig,
    release_update_state::{ReleaseUpdateStateFailure, ReleaseUpdateStateStore},
    release_update_watch::{
        AUTOMATIC_INTERVAL, FIRST_CHECK_DELAY, ReleaseUpdateStatus, SCHEDULE_TICK, UpdateNotice,
        WINDOW_RECHECK_AFTER, app_translocated, available_notification, due,
    },
};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const UPDATE_STATUS_EVENT: &str = "desktop://update-status";

#[derive(Clone)]
pub(crate) struct ReleaseUpdateRuntime {
    service: Option<Arc<ReleaseUpdateService>>,
}

impl ReleaseUpdateRuntime {
    pub(crate) fn new(
        app: AppHandle,
        config: Option<ReleaseUpdateConfig>,
    ) -> Result<Self, ReleaseUpdateFailure> {
        let Some(config) = config else {
            return Ok(Self { service: None });
        };
        let data_root = app
            .path()
            .app_data_dir()
            .map_err(|_| ReleaseUpdateFailure::state("desktop.update.data_path_failed"))?;
        let notice = UpdateNotice::new(data_root.join("update-notified"));
        let state = ReleaseUpdateStateStore::new(data_root.join("release-trust"));
        let current_version = app.package_info().version.to_string();
        state
            .reconcile_installation(&current_version)
            .map_err(ReleaseUpdateFailure::from_state)?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(3))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| ReleaseUpdateFailure::network("desktop.update.client_failed"))?;
        Ok(Self {
            service: Some(Arc::new(ReleaseUpdateService {
                app,
                client,
                config,
                current_version,
                state,
                notice,
                watch: Mutex::default(),
                checking: tokio::sync::Mutex::new(()),
            })),
        })
    }

    pub(crate) const fn configured(&self) -> bool {
        self.service.is_some()
    }

    /// 手动检查（设置里的按钮），渠道由人选。结果和自动检查记在同一处。
    pub(crate) async fn check(
        &self,
        channel: ReleaseChannel,
    ) -> Result<ReleaseUpdateCheck, ReleaseUpdateFailure> {
        let service = self.service()?;
        let _checking = service.checking.lock().await;
        service.check_now(channel, CheckOrigin::Manual).await
    }

    pub(crate) async fn install(
        &self,
        channel: ReleaseChannel,
        expected_sequence: u64,
    ) -> Result<(), ReleaseUpdateFailure> {
        self.service()?.install(channel, expected_sequence).await
    }

    /// 应用开着就定时查：启动 30 秒后第一次，之后每 4 小时。按墙上时间判断，睡醒以后几分钟内就查。
    pub(crate) fn start_schedule(&self) {
        let Some(service) = self.service.clone() else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            loop {
                service.check_if_due(AUTOMATIC_INTERVAL).await;
                tokio::time::sleep(SCHEDULE_TICK).await;
            }
        });
    }

    /// 窗口重新打开了：离上次检查超过 1 小时就马上查。
    pub(crate) fn window_shown(&self) {
        let Some(service) = self.service.clone() else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            service.check_if_due(WINDOW_RECHECK_AFTER).await;
        });
    }

    /// 上次检查的结果，网页加载时随快照一起拿。
    pub(crate) fn status(&self) -> Option<ReleaseUpdateStatus> {
        self.service
            .as_deref()
            .and_then(|service| service.watch().status.clone())
    }

    /// 已知可以装的新版本，托盘菜单重建时用。
    pub(crate) fn available_version(&self) -> Option<String> {
        self.service
            .as_deref()
            .and_then(|service| service.watch().tray_version.clone())
    }

    fn service(&self) -> Result<&ReleaseUpdateService, ReleaseUpdateFailure> {
        self.service
            .as_deref()
            .ok_or_else(|| ReleaseUpdateFailure::policy("desktop.update.unavailable"))
    }
}

struct ReleaseUpdateService {
    app: AppHandle,
    client: reqwest::Client,
    config: ReleaseUpdateConfig,
    current_version: String,
    state: ReleaseUpdateStateStore,
    notice: UpdateNotice,
    watch: Mutex<UpdateWatch>,
    /// 同一时间只查一次；手动检查排队等，自动检查碰上就跳过。安装时也拿着它，装的时候不查。
    checking: tokio::sync::Mutex<()>,
}

/// 检查的记录：上次什么时候查的（自动检查据此判断该不该查）、结果、托盘上挂着哪个版本。
#[derive(Default)]
struct UpdateWatch {
    last_attempt: Option<SystemTime>,
    status: Option<ReleaseUpdateStatus>,
    tray_version: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckOrigin {
    Automatic,
    Manual,
}

impl ReleaseUpdateService {
    fn watch(&self) -> MutexGuard<'_, UpdateWatch> {
        self.watch.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 离上次检查满了 `interval` 就按这一版所属的渠道查一次。正在安装、或者别的检查正在跑，就不查。
    async fn check_if_due(&self, interval: Duration) {
        let Ok(_checking) = self.checking.try_lock() else {
            return;
        };
        if !due(self.watch().last_attempt, SystemTime::now(), interval) {
            return;
        }
        let channel = default_channel(&self.current_version);
        let _ = self.check_now(channel, CheckOrigin::Automatic).await;
    }

    /// 查一次并记下结果。调用方要拿着 `checking`。
    async fn check_now(
        &self,
        channel: ReleaseChannel,
        origin: CheckOrigin,
    ) -> Result<ReleaseUpdateCheck, ReleaseUpdateFailure> {
        self.watch().last_attempt = Some(SystemTime::now());
        let outcome = self.inspect(channel).await;
        self.record(origin, channel, &outcome);
        outcome
    }

    /// 记下结果、告诉界面、写一行日志；可装的版本变了就重建托盘菜单，新版本第一次查到时提醒。
    fn record(
        &self,
        origin: CheckOrigin,
        channel: ReleaseChannel,
        outcome: &Result<ReleaseUpdateCheck, ReleaseUpdateFailure>,
    ) {
        let automatic = origin == CheckOrigin::Automatic;
        match outcome {
            Ok(check) if check.available => {
                tracing::info!(target_version = %check.target_version, automatic, "查到桌面端新版本");
            }
            Ok(_) => tracing::debug!(automatic, "桌面端已是最新版本"),
            Err(failure) => {
                tracing::warn!(error_code = failure.code(), automatic, "检查桌面端更新没成");
            }
        }
        let (status, tray_changed) = {
            let mut watch = self.watch();
            let status = ReleaseUpdateStatus::after(
                watch.status.as_ref(),
                channel_name(channel),
                outcome,
                now_unix_millis(),
            );
            let version = status.available_version().map(str::to_owned);
            let tray_changed = watch.tray_version != version;
            watch.tray_version = version;
            watch.status = Some(status.clone());
            (status, tray_changed)
        };
        let _ = self.app.emit(UPDATE_STATUS_EVENT, &status);
        if tray_changed {
            native_language::refresh_tray_menu(&self.app, status.available_version());
        }
        if let Some(version) = status.available_version() {
            self.announce(origin, version);
        }
    }

    /// 每个版本只提醒一次。手动查到的、或者窗口正开在前台，人已经看到了，只记下不发通知。
    fn announce(&self, origin: CheckOrigin, version: &str) {
        if !self.notice.claim(version)
            || origin == CheckOrigin::Manual
            || main_window_in_front(&self.app)
        {
            return;
        }
        let (title, body) = available_notification(native_language::language(&self.app), version);
        if let Err(error) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!(%error, "新版本的系统通知没发出去");
        }
    }

    async fn inspect(
        &self,
        channel: ReleaseChannel,
    ) -> Result<ReleaseUpdateCheck, ReleaseUpdateFailure> {
        match self.prepare(channel).await? {
            PreparedRelease::Current { sequence } => Ok(ReleaseUpdateCheck {
                available: false,
                channel: channel_name(channel),
                current_version: self.current_version.clone(),
                target_version: self.current_version.clone(),
                sequence,
                rollback: false,
            }),
            PreparedRelease::Update(prepared) => {
                let manifest = prepared.verified.manifest();
                Ok(ReleaseUpdateCheck {
                    available: true,
                    channel: channel_name(channel),
                    current_version: self.current_version.clone(),
                    target_version: manifest.version.clone(),
                    sequence: manifest.sequence,
                    rollback: manifest.rollback_from.is_some(),
                })
            }
        }
    }

    /// 装好就重启，不会返回；返回的都是没装成。装的时候不让自动检查插进来。
    async fn install(
        &self,
        channel: ReleaseChannel,
        expected_sequence: u64,
    ) -> Result<(), ReleaseUpdateFailure> {
        // 在只读位置运行时换不了自己，下载前就说清楚。
        if app_translocated() {
            return Err(ReleaseUpdateFailure::policy(
                "desktop.update.app_translocated",
            ));
        }
        let _checking = self.checking.lock().await;
        self.download_and_install(channel, expected_sequence).await
    }

    async fn download_and_install(
        &self,
        channel: ReleaseChannel,
        expected_sequence: u64,
    ) -> Result<(), ReleaseUpdateFailure> {
        let PreparedRelease::Update(prepared) = self.prepare(channel).await? else {
            return Err(ReleaseUpdateFailure::policy(
                "desktop.update.no_update_available",
            ));
        };
        let manifest = prepared.verified.manifest();
        if manifest.sequence != expected_sequence {
            return Err(ReleaseUpdateFailure::policy("desktop.update.plan_changed"));
        }

        // 下载几十 MB 时按钮不能一动不动：把进度发给界面，最多每 1% 一次。
        let mut progress =
            UpdateProgressReporter::new(self.app.clone(), prepared.artifact.byte_length);
        let bytes = prepared
            .update
            .download(|chunk, _| progress.downloaded(chunk), || {})
            .await
            .map_err(|_| ReleaseUpdateFailure::network("desktop.update.download_failed"))?;
        validate_download(&bytes, &prepared.artifact)?;
        progress.installing();
        self.state
            .record_pending(channel, manifest.sequence, &manifest.version)
            .map_err(ReleaseUpdateFailure::from_state)?;
        prepared
            .update
            .install(&bytes)
            .map_err(|_| ReleaseUpdateFailure::state("desktop.update.install_failed"))?;
        self.app.restart()
    }

    async fn prepare(
        &self,
        channel: ReleaseChannel,
    ) -> Result<PreparedRelease, ReleaseUpdateFailure> {
        let envelope = self.fetch_manifest(channel).await?;
        let trust_state = self
            .state
            .trust_state(channel, &self.current_version)
            .map_err(ReleaseUpdateFailure::from_state)?;
        let inspection = inspect_release(
            &envelope,
            self.config.trusted_key(),
            channel,
            &trust_state,
            now_unix_seconds()?,
        )
        .map_err(|error| ReleaseUpdateFailure::policy(manifest_failure_code(&error)))?;
        let ReleaseInspection::Update(verified) = inspection else {
            let ReleaseInspection::Current(manifest) = inspection else {
                unreachable!("发布检查只有当前版本和更新版本")
            };
            return Ok(PreparedRelease::Current {
                sequence: manifest.sequence,
            });
        };

        let manifest = verified.manifest();
        let endpoint = Url::parse(
            manifest
                .tauri_manifest_url
                .as_deref()
                .ok_or_else(|| ReleaseUpdateFailure::policy("desktop.update.metadata_missing"))?,
        )
        .map_err(|_| ReleaseUpdateFailure::policy("desktop.update.metadata_invalid"))?;
        let target_version = manifest.version.clone();
        let comparator_version = target_version.clone();
        let updater = self
            .app
            .updater_builder()
            .endpoints(vec![endpoint])
            .map_err(|_| ReleaseUpdateFailure::policy("desktop.update.metadata_invalid"))?
            .version_comparator(move |_current, remote| {
                remote.version.to_string() == comparator_version
            })
            .build()
            .map_err(|_| ReleaseUpdateFailure::policy("desktop.update.updater_unavailable"))?;
        let update = updater
            .check()
            .await
            .map_err(|_| ReleaseUpdateFailure::network("desktop.update.metadata_failed"))?
            .ok_or_else(|| ReleaseUpdateFailure::policy("desktop.update.metadata_mismatch"))?;
        if update.version != target_version {
            return Err(ReleaseUpdateFailure::policy(
                "desktop.update.metadata_mismatch",
            ));
        }
        let artifact = select_artifact(manifest.artifacts.as_slice(), &update)?;
        Ok(PreparedRelease::Update(Box::new(PreparedUpdate {
            artifact,
            update,
            verified,
        })))
    }

    async fn fetch_manifest(
        &self,
        channel: ReleaseChannel,
    ) -> Result<SignedReleaseManifest, ReleaseUpdateFailure> {
        let endpoint = match channel {
            ReleaseChannel::Stable => self.config.stable_url(),
            ReleaseChannel::Testing => self.config.testing_url(),
        };
        let response = self
            .client
            .get(endpoint.clone())
            .send()
            .await
            .map_err(|_| ReleaseUpdateFailure::network("desktop.update.manifest_network"))?
            .error_for_status()
            .map_err(|_| ReleaseUpdateFailure::network("desktop.update.manifest_network"))?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MANIFEST_BYTES as u64)
        {
            return Err(ReleaseUpdateFailure::policy(
                "desktop.update.manifest_too_large",
            ));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| ReleaseUpdateFailure::network("desktop.update.manifest_network"))?;
            if body.len().saturating_add(chunk.len()) > MAX_MANIFEST_BYTES {
                return Err(ReleaseUpdateFailure::policy(
                    "desktop.update.manifest_too_large",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| ReleaseUpdateFailure::policy("desktop.update.manifest_invalid"))
    }
}

const UPDATE_PROGRESS_EVENT: &str = "desktop://update-progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    phase: &'static str,
    downloaded_bytes: u64,
    total_bytes: u64,
}

/// 把下载进度发给界面；按整数百分比去重，避免每个数据块都刷一次。
struct UpdateProgressReporter {
    app: AppHandle,
    total_bytes: u64,
    downloaded_bytes: u64,
    reported_percent: u64,
}

impl UpdateProgressReporter {
    fn new(app: AppHandle, total_bytes: u64) -> Self {
        Self {
            app,
            total_bytes,
            downloaded_bytes: 0,
            reported_percent: u64::MAX,
        }
    }

    fn downloaded(&mut self, chunk: usize) {
        self.downloaded_bytes = self.downloaded_bytes.saturating_add(chunk as u64);
        let percent = percent_of(self.downloaded_bytes, self.total_bytes);
        if percent != self.reported_percent {
            self.reported_percent = percent;
            self.emit("downloading");
        }
    }

    fn installing(&mut self) {
        self.emit("installing");
    }

    fn emit(&self, phase: &'static str) {
        let _ = self.app.emit(
            UPDATE_PROGRESS_EVENT,
            UpdateProgress {
                phase,
                downloaded_bytes: self.downloaded_bytes,
                total_bytes: self.total_bytes,
            },
        );
    }
}

fn percent_of(part: u64, total: u64) -> u64 {
    part.saturating_mul(100).checked_div(total).unwrap_or(0)
}

enum PreparedRelease {
    Current { sequence: u64 },
    Update(Box<PreparedUpdate>),
}

struct PreparedUpdate {
    artifact: ReleaseArtifact,
    update: Update,
    verified: VerifiedRelease,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseUpdateCheck {
    available: bool,
    channel: &'static str,
    current_version: String,
    target_version: String,
    sequence: u64,
    rollback: bool,
}

impl ReleaseUpdateCheck {
    pub(crate) const fn available(&self) -> bool {
        self.available
    }

    pub(crate) fn target_version(&self) -> &str {
        &self.target_version
    }

    #[cfg(test)]
    pub(crate) fn for_tests(available: bool, current: &str, target: &str) -> Self {
        Self {
            available,
            channel: "testing",
            current_version: current.to_owned(),
            target_version: target.to_owned(),
            sequence: 1,
            rollback: false,
        }
    }
}

/// 清单过期单给一个错误码：两次发版隔了一周以上就会这样，下次发版就好，不是故障。
const fn manifest_failure_code(error: &ReleaseManifestError) -> &'static str {
    match error {
        ReleaseManifestError::Expired => "desktop.update.manifest_expired",
        _ => "desktop.update.manifest_rejected",
    }
}

fn select_artifact(
    artifacts: &[ReleaseArtifact],
    update: &Update,
) -> Result<ReleaseArtifact, ReleaseUpdateFailure> {
    let selected = artifacts.iter().find(|artifact| {
        artifact.kind == ArtifactKind::Desktop
            && artifact.platform == update.target
            && Url::parse(&artifact.url).is_ok_and(|url| url == update.download_url)
    });
    selected
        .cloned()
        .ok_or_else(|| ReleaseUpdateFailure::policy("desktop.update.artifact_mismatch"))
}

fn validate_download(bytes: &[u8], artifact: &ReleaseArtifact) -> Result<(), ReleaseUpdateFailure> {
    let length = u64::try_from(bytes.len())
        .map_err(|_| ReleaseUpdateFailure::policy("desktop.update.artifact_size_mismatch"))?;
    if length != artifact.byte_length {
        return Err(ReleaseUpdateFailure::policy(
            "desktop.update.artifact_size_mismatch",
        ));
    }
    let digest =
        Sha256::digest(bytes)
            .iter()
            .fold(String::with_capacity(64), |mut output, byte| {
                write!(output, "{byte:02x}").expect("写入 String 不会失败");
                output
            });
    if digest != artifact.sha256 {
        return Err(ReleaseUpdateFailure::policy(
            "desktop.update.artifact_digest_mismatch",
        ));
    }
    Ok(())
}

fn now_unix_seconds() -> Result<u64, ReleaseUpdateFailure> {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| ReleaseUpdateFailure::state("desktop.update.clock_invalid"))
}

fn now_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// 自动检查走这一版所属的渠道，和网页层以前的规则一样：版本号带 `-` 的是测试版。
pub(crate) fn default_channel(version: &str) -> ReleaseChannel {
    if version.contains('-') {
        ReleaseChannel::Testing
    } else {
        ReleaseChannel::Stable
    }
}

/// 主窗口开着、又在前台：窗口里的提示人已经看得到，不用再发系统通知。
fn main_window_in_front(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|window| {
        window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
    })
}

const fn channel_name(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => "stable",
        ReleaseChannel::Testing => "testing",
    }
}

pub(crate) fn parse_channel(value: &str) -> Result<ReleaseChannel, ReleaseUpdateFailure> {
    match value {
        "stable" => Ok(ReleaseChannel::Stable),
        "testing" => Ok(ReleaseChannel::Testing),
        _ => Err(ReleaseUpdateFailure::policy(
            "desktop.update.channel_invalid",
        )),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReleaseUpdateFailure {
    code: &'static str,
    retryable: bool,
}

impl ReleaseUpdateFailure {
    const fn network(code: &'static str) -> Self {
        Self {
            code,
            retryable: true,
        }
    }

    const fn policy(code: &'static str) -> Self {
        Self {
            code,
            retryable: false,
        }
    }

    const fn state(code: &'static str) -> Self {
        Self {
            code,
            retryable: true,
        }
    }

    fn from_state(failure: ReleaseUpdateStateFailure) -> Self {
        let code = failure.code();
        drop(failure);
        Self::state(code)
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }

    pub(crate) const fn retryable(self) -> bool {
        self.retryable
    }

    #[cfg(test)]
    pub(crate) const fn for_tests(code: &'static str, retryable: bool) -> Self {
        Self { code, retryable }
    }
}

#[cfg(test)]
mod tests {
    use super::percent_of;

    #[test]
    fn 进度百分比不除零且封顶于整数() {
        assert_eq!(percent_of(0, 0), 0);
        assert_eq!(percent_of(5, 0), 0);
        assert_eq!(percent_of(1, 3), 33);
        assert_eq!(percent_of(3, 3), 100);
        assert_eq!(percent_of(u64::MAX, 2), u64::MAX / 2);
    }

    use super::*;

    #[test]
    fn 下载摘要和长度必须同时匹配() {
        let bytes = b"signed updater bytes";
        let digest =
            Sha256::digest(bytes)
                .iter()
                .fold(String::with_capacity(64), |mut output, byte| {
                    write!(output, "{byte:02x}").expect("写入 String 不会失败");
                    output
                });
        let mut artifact = ReleaseArtifact {
            name: "desktop".to_owned(),
            kind: ArtifactKind::Desktop,
            platform: "windows-x86_64".to_owned(),
            url: "https://releases.example/update.exe".to_owned(),
            sha256: digest,
            byte_length: u64::try_from(bytes.len()).expect("测试长度必须可表示"),
            sbom_url: "https://releases.example/update.cdx.json".to_owned(),
            signature_url: "https://releases.example/update.sig".to_owned(),
        };

        assert!(validate_download(bytes, &artifact).is_ok());
        artifact.byte_length += 1;
        assert_eq!(
            validate_download(bytes, &artifact)
                .expect_err("篡改长度必须失败")
                .code(),
            "desktop.update.artifact_size_mismatch"
        );
        artifact.byte_length -= 1;
        artifact.sha256 = "0".repeat(64);
        assert_eq!(
            validate_download(bytes, &artifact)
                .expect_err("篡改摘要必须失败")
                .code(),
            "desktop.update.artifact_digest_mismatch"
        );
    }

    #[test]
    fn 自动检查按版本号定渠道() {
        assert_eq!(default_channel("0.1.0-alpha.67"), ReleaseChannel::Testing);
        assert_eq!(default_channel("1.0.0"), ReleaseChannel::Stable);
    }

    #[test]
    fn 清单过期单给一个错误码() {
        assert_eq!(
            manifest_failure_code(&ReleaseManifestError::Expired),
            "desktop.update.manifest_expired"
        );
        assert_eq!(
            manifest_failure_code(&ReleaseManifestError::InvalidSignature),
            "desktop.update.manifest_rejected"
        );
    }

    #[test]
    fn 渠道解析拒绝任意字符串() {
        assert!(matches!(
            parse_channel("stable"),
            Ok(ReleaseChannel::Stable)
        ));
        assert_eq!(
            parse_channel("nightly")
                .expect_err("未知渠道必须失败")
                .code(),
            "desktop.update.channel_invalid"
        );
    }
}
