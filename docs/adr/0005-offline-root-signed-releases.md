# ADR 0005: Offline-root signed releases

- Status: Accepted
- Date: 2026-08-26

## Context

Agent Room distributes OCI images, native Bridge binaries, desktop updaters, and framework plugins. A compromised CI token or mutable release URL must not authorize arbitrary updates or downgrade a client to a vulnerable version.

## Decision

Every release asset carries a digest, length, CycloneDX SBOM, and Sigstore evidence. CI creates candidates, but an offline Ed25519 root signs the final stable or testing manifest. Clients enforce channel, expiry, monotonic sequence, exact artifact identity, and explicit rollback authorization. Desktop updates also verify the Tauri signature.

Promotion follows database expansion, compatible server, clients, observation, and legacy contraction. The offline private key never enters CI.

## Consequences

- CI compromise alone cannot create a trusted channel manifest.
- Key ceremony, protected environments, and recovery procedures become release prerequisites.
- Rollbacks are possible but auditable and narrowly declared.
- Releases take more operational discipline than attaching files to a tag.

## Revisit when

The trust root may be rotated or moved to an equivalent threshold/offline system, but clients must retain anti-downgrade and exact-artifact verification.
