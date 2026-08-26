mod error;
mod model;
mod verifier;

pub use error::{ReleaseManifestError, ReleaseManifestResult};
pub use model::{
    ArtifactKind, ReleaseArtifact, ReleaseChannel, ReleaseInspection, ReleaseManifest,
    ReleaseTrustState, SignedReleaseManifest, TrustedReleaseKey, VerifiedRelease,
};
pub use verifier::{
    MAX_CLOCK_SKEW_SECONDS, MAX_MANIFEST_LIFETIME_SECONDS, inspect_release,
    validate_release_document, verify_release,
};
