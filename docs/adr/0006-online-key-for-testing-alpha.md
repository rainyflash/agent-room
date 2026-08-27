# ADR 0006: Online key for the testing Alpha channel

- Status: Accepted
- Date: 2026-08-27
- Supersedes: ADR 0005 for the `testing` Alpha channel only

## Context

ADR 0005 correctly defines the stable-release target: an offline Ed25519 root and independent approval prevent CI compromise from becoming a trusted client update. Applying that full operating model to the first single-maintainer Alpha, however, blocks test distribution without improving the product code being exercised. The project has no offline signing device or second release maintainer yet, and the Alpha is explicitly unsupported, unsigned by Authenticode, and restricted to the `testing` channel.

## Decision

The `testing` Alpha channel uses a dedicated Ed25519 private key stored only in the protected GitHub `release-candidate` Environment. The candidate workflow signs `release.json`, verifies the resulting envelope against repository public-key variables, deletes the runner copy, and creates a Draft Release. Final publication still requires the ordered server-promotion record and the `public-release` environment gate.

The testing key is never trusted for `stable`. Before stable distribution, clients must receive a distinct stable public key through an explicitly reviewed trust migration, and ADR 0005's offline-root and independent-approval requirements apply unchanged.

## Consequences

- A compromise that can execute the protected candidate workflow and read Environment Secrets can forge testing-channel manifests.
- Tauri signatures, Sigstore identity, artifact hashes, SBOMs, monotonic sequence checks, explicit rollback, and ordered promotion still provide independent evidence and failure boundaries.
- A single maintainer can publish the first Alpha without pretending that self-approval is a two-person security control.
- testing-key rotation may require an out-of-band Alpha client reinstall; this is acceptable for an unsupported test channel.
- stable remains blocked until its separate offline trust root and approval policy exist.

## Revisit when

Revisit before the first stable candidate, or earlier if the testing channel gains enough users that forced reinstall is no longer an acceptable key-recovery path.
