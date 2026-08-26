use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    Stable,
    Testing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    OciImage,
    Bridge,
    Desktop,
    CodexPlugin,
    UpdateManifest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseArtifact {
    pub name: String,
    pub kind: ArtifactKind,
    pub platform: String,
    pub url: String,
    pub sha256: String,
    pub byte_length: u64,
    pub sbom_url: String,
    pub signature_url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub channel: ReleaseChannel,
    pub sequence: u64,
    pub version: String,
    pub published_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub rollback_from: Option<String>,
    pub tauri_manifest_url: Option<String>,
    pub artifacts: Vec<ReleaseArtifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedReleaseManifest {
    pub algorithm: String,
    pub key_id: String,
    pub payload: String,
    pub signature: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedReleaseKey {
    pub key_id: String,
    pub public_key: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseTrustState {
    pub channel: ReleaseChannel,
    pub highest_sequence: u64,
    pub installed_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRelease {
    manifest: ReleaseManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseInspection {
    Current(ReleaseManifest),
    Update(VerifiedRelease),
}

impl VerifiedRelease {
    pub fn manifest(&self) -> &ReleaseManifest {
        &self.manifest
    }

    pub fn commit_success(&self) -> ReleaseTrustState {
        ReleaseTrustState {
            channel: self.manifest.channel,
            highest_sequence: self.manifest.sequence,
            installed_version: self.manifest.version.clone(),
        }
    }

    pub(crate) fn new(manifest: ReleaseManifest) -> Self {
        Self { manifest }
    }
}
