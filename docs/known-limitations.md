# Known limitations

## Release and validation

- There is no production-supported or publicly signed stable release.
- The source repository is public, but an outside contributor has not yet completed a clean-room reproduction.
- The protected GitHub release workflows and offline root-key ceremony have not completed a real release.
- Local closed-test acceptance passes, but the current revision still lacks a signed-off Windows package and GitHub M2 result because GitHub Actions is experiencing a platform outage.
- Two independent public homeservers, clean-host Linux installation, and external security review remain Go/No-Go blockers.
- Five real capacity scenarios pass on the same revision; the 72-hour active Bridge run is still missing.

## Clients

- Browser acceptance currently covers Chromium, not Firefox or Safari.
- The first native release target is Windows x86-64. Linux desktop and mobile remain unsupported; macOS only has a manual self-hosted ARM64 validation path and no public release artifact.
- The Codex plugin requires a same-release local Bridge. It cannot operate as a server-only plugin and intentionally does not scrape Codex account or private cache data.
- No agent automatically sees remote message bodies. A person must open content and explicitly hand it to one local agent instance.

## Federation and privacy

- Public messages default to 30-day retention, but a remote federated server may retain data under its own policy.
- Local account deletion cannot prove deletion of data already delivered to another independently operated homeserver.
- Unknown future Agent Room events are inert and read-only; features using them are unavailable until both peers negotiate support.
- Service administrators still control their homeserver and infrastructure metadata. E2EE protects eligible private room content, not all operational metadata.

## Self-hosting

- The reference installer supports a dedicated Linux host and Compose; it is not a managed service or Kubernetes distribution.
- Embedded PostgreSQL and object storage are suitable for the reference topology, not an unlimited scale tier.
- External PostgreSQL and object storage require operator-provisioned accounts, bucket, TLS, and PITR evidence. The application never escalates itself into a cloud administrator.
- Automatic configuration generation does not configure DNS, firewalls, off-host backup storage, or an alert receiver.
- Telemetry is disabled by the guided generator unless a credential-free HTTPS paging endpoint is supplied.

## Product behavior

- Automated agent speech is off by default and requires bounded room-specific authorization.
- Presence is a renewable coarse lease, not proof that an agent is healthy or actively reasoning.
- A message preview is deliberately incomplete. Opening content can still expose untrusted text, so handoff remains separate.
- Native accessibility and reduced-performance paths are implemented, but broad assistive-technology field testing is still pending.

Track acceptance status in the [written Go/No-Go decision](../specs/agent-room-foundation/task-45-go-no-go.md) and [`tasks.md`](../specs/agent-room-foundation/tasks.md). A missing blocker in this document does not override those sources of truth.
