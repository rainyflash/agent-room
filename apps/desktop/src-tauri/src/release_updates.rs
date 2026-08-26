use std::{
    fmt::Write as _,
    sync::Arc,
    time::{Duration, SystemTime},
};

use agent_room_release_manifest::{
    ArtifactKind, ReleaseArtifact, ReleaseChannel, ReleaseInspection, SignedReleaseManifest,
    VerifiedRelease, inspect_release,
};
use futures_util::StreamExt as _;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Manager as _};
use tauri_plugin_updater::{Update, UpdaterExt as _};
use url::Url;

use crate::{
    release_update_config::ReleaseUpdateConfig,
    release_update_state::{ReleaseUpdateStateFailure, ReleaseUpdateStateStore},
};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

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
        let state_root = app
            .path()
            .app_data_dir()
            .map_err(|_| ReleaseUpdateFailure::state("desktop.update.data_path_failed"))?
            .join("release-trust");
        let state = ReleaseUpdateStateStore::new(state_root);
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
            })),
        })
    }

    pub(crate) const fn configured(&self) -> bool {
        self.service.is_some()
    }

    pub(crate) async fn check(
        &self,
        channel: ReleaseChannel,
    ) -> Result<ReleaseUpdateCheck, ReleaseUpdateFailure> {
        self.service()?.check(channel).await
    }

    pub(crate) async fn install(
        &self,
        channel: ReleaseChannel,
        expected_sequence: u64,
    ) -> Result<(), ReleaseUpdateFailure> {
        self.service()?.install(channel, expected_sequence).await
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
}

impl ReleaseUpdateService {
    async fn check(
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

    async fn install(
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

        let bytes = prepared
            .update
            .download(|_, _| {}, || {})
            .await
            .map_err(|_| ReleaseUpdateFailure::network("desktop.update.download_failed"))?;
        validate_download(&bytes, &prepared.artifact)?;
        self.state
            .record_pending(channel, manifest.sequence, &manifest.version)
            .map_err(ReleaseUpdateFailure::from_state)?;
        prepared
            .update
            .install(&bytes)
            .map_err(|_| ReleaseUpdateFailure::state("desktop.update.install_failed"))
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
        .map_err(|_| ReleaseUpdateFailure::policy("desktop.update.manifest_rejected"))?;
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

enum PreparedRelease {
    Current { sequence: u64 },
    Update(Box<PreparedUpdate>),
}

struct PreparedUpdate {
    artifact: ReleaseArtifact,
    update: Update,
    verified: VerifiedRelease,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseUpdateCheck {
    available: bool,
    channel: &'static str,
    current_version: String,
    target_version: String,
    sequence: u64,
    rollback: bool,
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
}

#[cfg(test)]
mod tests {
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
