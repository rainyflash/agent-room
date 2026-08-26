use std::collections::HashSet;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;

use crate::{
    ArtifactKind, ReleaseChannel, ReleaseInspection, ReleaseManifest, ReleaseManifestError,
    ReleaseManifestResult, ReleaseTrustState, SignedReleaseManifest, TrustedReleaseKey,
    VerifiedRelease,
};

pub const MAX_CLOCK_SKEW_SECONDS: u64 = 300;
pub const MAX_MANIFEST_LIFETIME_SECONDS: u64 = 30 * 24 * 60 * 60;

/// 验证准备签名的发布文档是否满足静态策略。
///
/// # Errors
///
/// 当架构、时效、版本或产物元数据无效时返回错误。
pub fn validate_release_document(
    manifest: &ReleaseManifest,
    now_unix_seconds: u64,
) -> ReleaseManifestResult<()> {
    if manifest.schema_version != 1 {
        return Err(ReleaseManifestError::UnsupportedSchema);
    }
    validate_time_window(manifest, now_unix_seconds)?;
    Version::parse(&manifest.version).map_err(|_| ReleaseManifestError::InvalidVersion)?;
    if let Some(rollback_from) = &manifest.rollback_from {
        Version::parse(rollback_from).map_err(|_| ReleaseManifestError::InvalidVersion)?;
    }
    validate_artifacts(manifest)
}

/// 检查已签名清单，并区分“已经是当前版本”和“需要更新”。
///
/// # Errors
///
/// 当签名、渠道、时效、序号或版本迁移策略无效时返回错误。
pub fn inspect_release(
    envelope: &SignedReleaseManifest,
    trusted_key: &TrustedReleaseKey,
    expected_channel: ReleaseChannel,
    trust_state: &ReleaseTrustState,
    now_unix_seconds: u64,
) -> ReleaseManifestResult<ReleaseInspection> {
    let payload = authenticate_envelope(envelope, trusted_key)?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&payload).map_err(|_| ReleaseManifestError::InvalidPayload)?;
    if manifest.channel != expected_channel || trust_state.channel != expected_channel {
        return Err(ReleaseManifestError::ChannelMismatch);
    }
    validate_release_document(&manifest, now_unix_seconds)?;

    let candidate =
        Version::parse(&manifest.version).map_err(|_| ReleaseManifestError::InvalidVersion)?;
    let installed = Version::parse(&trust_state.installed_version)
        .map_err(|_| ReleaseManifestError::InvalidVersion)?;
    if candidate == installed {
        if manifest.sequence < trust_state.highest_sequence {
            return Err(ReleaseManifestError::StaleSequence);
        }
        if manifest.rollback_from.is_some() {
            return Err(ReleaseManifestError::UnauthorizedRollback);
        }
        return Ok(ReleaseInspection::Current(manifest));
    }

    validate_manifest(&manifest, expected_channel, trust_state, now_unix_seconds)?;
    Ok(ReleaseInspection::Update(VerifiedRelease::new(manifest)))
}

/// 验证离线签名的发布清单，但不改变本地可信状态。
///
/// # Errors
///
/// 当签名、渠道、时效、版本迁移或产物元数据不满足发布策略时返回错误。
pub fn verify_release(
    envelope: &SignedReleaseManifest,
    trusted_key: &TrustedReleaseKey,
    expected_channel: ReleaseChannel,
    trust_state: &ReleaseTrustState,
    now_unix_seconds: u64,
) -> ReleaseManifestResult<VerifiedRelease> {
    let payload = authenticate_envelope(envelope, trusted_key)?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&payload).map_err(|_| ReleaseManifestError::InvalidPayload)?;

    validate_manifest(&manifest, expected_channel, trust_state, now_unix_seconds)?;
    Ok(VerifiedRelease::new(manifest))
}

fn authenticate_envelope(
    envelope: &SignedReleaseManifest,
    trusted_key: &TrustedReleaseKey,
) -> ReleaseManifestResult<Vec<u8>> {
    if envelope.algorithm != "Ed25519" {
        return Err(ReleaseManifestError::UnsupportedAlgorithm);
    }
    if envelope.key_id != trusted_key.key_id {
        return Err(ReleaseManifestError::UntrustedKey);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(&envelope.payload)
        .map_err(|_| ReleaseManifestError::InvalidPayloadEncoding)?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(&envelope.signature)
        .map_err(|_| ReleaseManifestError::InvalidSignatureEncoding)?;
    let signature = Signature::try_from(signature_bytes.as_slice())
        .map_err(|_| ReleaseManifestError::InvalidSignatureEncoding)?;
    let public_key = VerifyingKey::from_bytes(&trusted_key.public_key)
        .map_err(|_| ReleaseManifestError::UntrustedKey)?;
    public_key
        .verify_strict(&payload, &signature)
        .map_err(|_| ReleaseManifestError::InvalidSignature)?;

    Ok(payload)
}

fn validate_manifest(
    manifest: &ReleaseManifest,
    expected_channel: ReleaseChannel,
    trust_state: &ReleaseTrustState,
    now_unix_seconds: u64,
) -> ReleaseManifestResult<()> {
    if manifest.channel != expected_channel || trust_state.channel != expected_channel {
        return Err(ReleaseManifestError::ChannelMismatch);
    }
    validate_release_document(manifest, now_unix_seconds)?;
    if manifest.sequence <= trust_state.highest_sequence {
        return Err(ReleaseManifestError::StaleSequence);
    }
    validate_version_transition(manifest, trust_state)?;
    Ok(())
}

fn validate_time_window(
    manifest: &ReleaseManifest,
    now_unix_seconds: u64,
) -> ReleaseManifestResult<()> {
    if manifest.published_at_unix_seconds > now_unix_seconds.saturating_add(MAX_CLOCK_SKEW_SECONDS)
    {
        return Err(ReleaseManifestError::PublishedInFuture);
    }
    if manifest.expires_at_unix_seconds <= now_unix_seconds {
        return Err(ReleaseManifestError::Expired);
    }
    let lifetime = manifest
        .expires_at_unix_seconds
        .checked_sub(manifest.published_at_unix_seconds)
        .ok_or(ReleaseManifestError::InvalidLifetime)?;
    if lifetime == 0 || lifetime > MAX_MANIFEST_LIFETIME_SECONDS {
        return Err(ReleaseManifestError::InvalidLifetime);
    }
    Ok(())
}

fn validate_version_transition(
    manifest: &ReleaseManifest,
    trust_state: &ReleaseTrustState,
) -> ReleaseManifestResult<()> {
    let candidate =
        Version::parse(&manifest.version).map_err(|_| ReleaseManifestError::InvalidVersion)?;
    let installed = Version::parse(&trust_state.installed_version)
        .map_err(|_| ReleaseManifestError::InvalidVersion)?;

    if candidate > installed {
        if manifest.rollback_from.is_some() {
            return Err(ReleaseManifestError::UnauthorizedRollback);
        }
        return Ok(());
    }
    if candidate == installed {
        return Err(ReleaseManifestError::VersionNotNewer);
    }
    if manifest.rollback_from.as_deref() != Some(trust_state.installed_version.as_str()) {
        return Err(ReleaseManifestError::UnauthorizedRollback);
    }
    Ok(())
}

fn validate_artifacts(manifest: &ReleaseManifest) -> ReleaseManifestResult<()> {
    if manifest.artifacts.is_empty() {
        return Err(ReleaseManifestError::MissingArtifacts);
    }

    let mut identities = HashSet::new();
    let mut contains_desktop = false;
    for artifact in &manifest.artifacts {
        if !identities.insert((
            artifact.kind,
            artifact.platform.as_str(),
            artifact.name.as_str(),
        )) {
            return Err(ReleaseManifestError::DuplicateArtifact);
        }
        if !is_artifact_name(&artifact.name) {
            return Err(ReleaseManifestError::InvalidArtifactName);
        }
        contains_desktop |= artifact.kind == ArtifactKind::Desktop;
        if !is_artifact_url(&artifact.url) {
            return Err(ReleaseManifestError::InvalidArtifactUrl);
        }
        if !is_lowercase_sha256(&artifact.sha256) {
            return Err(ReleaseManifestError::InvalidArtifactDigest);
        }
        if artifact.byte_length == 0 {
            return Err(ReleaseManifestError::InvalidArtifactSize);
        }
        if !is_https_url(&artifact.sbom_url) || !is_https_url(&artifact.signature_url) {
            return Err(ReleaseManifestError::InvalidAttestationUrl);
        }
    }

    match (contains_desktop, manifest.tauri_manifest_url.as_deref()) {
        (true, None) => Err(ReleaseManifestError::MissingTauriManifest),
        (_, Some(url)) if !is_https_url(url) => Err(ReleaseManifestError::InvalidTauriManifestUrl),
        _ => Ok(()),
    }
}

fn is_artifact_url(value: &str) -> bool {
    is_https_url(value) || is_oci_url(value)
}

fn is_https_url(value: &str) -> bool {
    value
        .strip_prefix("https://")
        .is_some_and(|remainder| !remainder.is_empty() && !remainder.contains(char::is_whitespace))
}

fn is_oci_url(value: &str) -> bool {
    value
        .strip_prefix("oci://")
        .is_some_and(|remainder| !remainder.is_empty() && !remainder.contains(char::is_whitespace))
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_artifact_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;
    use crate::{ReleaseArtifact, SignedReleaseManifest};

    const NOW: u64 = 1_800_000_000;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn trusted_key() -> TrustedReleaseKey {
        TrustedReleaseKey {
            key_id: "release-2026".to_owned(),
            public_key: signing_key().verifying_key().to_bytes(),
        }
    }

    fn trust_state() -> ReleaseTrustState {
        ReleaseTrustState {
            channel: ReleaseChannel::Stable,
            highest_sequence: 40,
            installed_version: "1.4.0".to_owned(),
        }
    }

    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema_version: 1,
            channel: ReleaseChannel::Stable,
            sequence: 41,
            version: "1.5.0".to_owned(),
            published_at_unix_seconds: NOW - 60,
            expires_at_unix_seconds: NOW + 86_400,
            rollback_from: None,
            tauri_manifest_url: Some("https://releases.example/tauri/stable.json".to_owned()),
            artifacts: vec![ReleaseArtifact {
                name: "desktop".to_owned(),
                kind: ArtifactKind::Desktop,
                platform: "windows-x86_64".to_owned(),
                url: "https://releases.example/agent-room_1.5.0_x64.exe".to_owned(),
                sha256: "a".repeat(64),
                byte_length: 1_024,
                sbom_url: "https://releases.example/agent-room_1.5.0.cdx.json".to_owned(),
                signature_url: "https://releases.example/agent-room_1.5.0.exe.sig".to_owned(),
            }],
        }
    }

    fn sign(manifest: &ReleaseManifest) -> SignedReleaseManifest {
        let payload = serde_json::to_vec(manifest).expect("测试清单必须可序列化");
        let signature = signing_key().sign(&payload);
        SignedReleaseManifest {
            algorithm: "Ed25519".to_owned(),
            key_id: "release-2026".to_owned(),
            payload: URL_SAFE_NO_PAD.encode(payload),
            signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        }
    }

    #[test]
    fn accepts_authenticated_upgrade_and_commits_only_on_success() {
        let envelope = sign(&manifest());
        let before = trust_state();

        let verified = verify_release(
            &envelope,
            &trusted_key(),
            ReleaseChannel::Stable,
            &before,
            NOW,
        )
        .expect("有效升级必须通过");

        assert_eq!(before.highest_sequence, 40);
        assert_eq!(verified.commit_success().highest_sequence, 41);
        assert_eq!(verified.commit_success().installed_version, "1.5.0");
    }

    #[test]
    fn rejects_tampered_payload() {
        let mut envelope = sign(&manifest());
        let mut payload = URL_SAFE_NO_PAD
            .decode(&envelope.payload)
            .expect("测试载荷必须可解码");
        payload[0] ^= 1;
        envelope.payload = URL_SAFE_NO_PAD.encode(payload);

        let result = verify_release(
            &envelope,
            &trusted_key(),
            ReleaseChannel::Stable,
            &trust_state(),
            NOW,
        );

        assert_eq!(result, Err(ReleaseManifestError::InvalidSignature));
    }

    #[test]
    fn rejects_replayed_or_downgraded_release() {
        let mut replayed = manifest();
        replayed.sequence = 40;
        assert_eq!(
            verify_release(
                &sign(&replayed),
                &trusted_key(),
                ReleaseChannel::Stable,
                &trust_state(),
                NOW,
            ),
            Err(ReleaseManifestError::StaleSequence)
        );

        let mut downgrade = manifest();
        downgrade.version = "1.3.0".to_owned();
        assert_eq!(
            verify_release(
                &sign(&downgrade),
                &trusted_key(),
                ReleaseChannel::Stable,
                &trust_state(),
                NOW,
            ),
            Err(ReleaseManifestError::UnauthorizedRollback)
        );
    }

    #[test]
    fn accepts_only_explicitly_authorized_rollback() {
        let mut rollback = manifest();
        rollback.version = "1.3.1".to_owned();
        rollback.rollback_from = Some("1.4.0".to_owned());

        let verified = verify_release(
            &sign(&rollback),
            &trusted_key(),
            ReleaseChannel::Stable,
            &trust_state(),
            NOW,
        )
        .expect("被离线密钥明确授权的回滚必须通过");

        assert_eq!(verified.commit_success().installed_version, "1.3.1");
    }

    #[test]
    fn rejects_expired_manifest() {
        let mut expired = manifest();
        expired.expires_at_unix_seconds = NOW;

        assert_eq!(
            verify_release(
                &sign(&expired),
                &trusted_key(),
                ReleaseChannel::Stable,
                &trust_state(),
                NOW,
            ),
            Err(ReleaseManifestError::Expired)
        );
    }

    #[test]
    fn rejects_invalid_artifact_metadata() {
        let mut invalid = manifest();
        invalid.artifacts[0].sha256 = "ABC".to_owned();

        assert_eq!(
            verify_release(
                &sign(&invalid),
                &trusted_key(),
                ReleaseChannel::Stable,
                &trust_state(),
                NOW,
            ),
            Err(ReleaseManifestError::InvalidArtifactDigest)
        );
    }

    #[test]
    fn reports_authenticated_current_release_without_advancing_state() {
        let mut current = manifest();
        current.version = "1.4.0".to_owned();
        current.sequence = 40;

        let inspection = inspect_release(
            &sign(&current),
            &trusted_key(),
            ReleaseChannel::Stable,
            &trust_state(),
            NOW,
        )
        .expect("当前签名版本必须能被识别");

        assert!(matches!(inspection, ReleaseInspection::Current(value) if value == current));
    }
}
