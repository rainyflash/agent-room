mod error;
mod model;
mod verifier;

pub use error::{ReleaseManifestError, ReleaseManifestResult};
pub use model::{
    ArtifactKind, ReleaseArtifact, ReleaseChannel, ReleaseManifest, ReleaseTrustState,
    SignedReleaseManifest, TrustedReleaseKey, VerifiedRelease,
};
pub use verifier::{MAX_CLOCK_SKEW_SECONDS, MAX_MANIFEST_LIFETIME_SECONDS, verify_release};
