# Compatibility and supported platforms

This matrix describes engineering coverage in the repository. It is not a production support promise; no public stable release has passed the Go/No-Go gate.

## Release-train compatibility

| Component                         | Compatibility rule                     | Failure behavior                                                              |
| --------------------------------- | -------------------------------------- | ----------------------------------------------------------------------------- |
| Control plane and database schema | Ordered, additive migrations first     | Startup/migration fails rather than silently skipping required schema         |
| Web/desktop cloud client and API  | Overlapping capability window          | Older clients ignore additive endpoints; newer clients surface missing APIs   |
| Desktop and bundled Bridge        | Same release artifact                  | Cloud UI remains usable; local runtime actions fail closed                    |
| Generic MCP server and Bridge     | Same release; IPC `1.0` must negotiate | MCP reports `bridge.ipc.version_incompatible` and does not load partial tools |
| Codex/Claude/Cursor adapters      | Configure the bundled same-release MCP | The desktop reports a bounded plan or conflict and does not overwrite blindly |
| Federated Agent Room peers        | Protocol `2.0` or previous major `1.0` | Newest common version is selected; unknown events are bounded read-only data  |

Do not combine files from separate release archives. Stable and testing channels have independent signed manifests and monotonic sequence state.

The Web client and the Tauri desktop shell use the same cloud ports and domain model. The desktop does not need its co-installed Bridge to browse cloud state. A release may add database columns, tables, and endpoints before a client consumes them, but it must not remove or reinterpret an existing contract in the same promotion. Database rollback is intentionally asymmetric: roll back the compatible application image and leave additive schema in place; never run a destructive down-migration during an incident.

## Client platforms

| Platform                                      | Engineering status                                                                                  | Public support status                        |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| Chromium-based desktop browser                | Automated multi-account Playwright acceptance without a local Bridge                                | Not yet supported for production             |
| Windows x86-64 desktop + Bridge + generic MCP | Real Tauri/WebView2 cloud acceptance with Bridge offline, plus install/runtime/uninstall acceptance | `0.1.0-alpha.15` public prerelease published |
| macOS arm64                                   | Manual maintainer-owned self-hosted build path only                                                 | Unsupported                                  |
| macOS x86-64                                  | No maintained build or release path                                                                 | Unsupported                                  |
| Linux desktop                                 | Workspace compilation only; no release bundle                                                       | Unsupported                                  |
| Firefox and Safari                            | No browser acceptance matrix yet                                                                    | Unsupported                                  |
| iOS and Android                               | No native client                                                                                    | Unsupported                                  |

The Web application has responsive and reduced-performance modes, but only the stated Chromium path is currently acceptance-tested.

The cloud-first source candidate has passed local same-revision validation but has not yet been promoted to production or published as the next Windows prerelease. Production must allow the exact Tauri origin `http://tauri.localhost`; wildcard origins are forbidden when credentials are enabled.

The first Windows bundle detects and configures Codex, Claude Code, and Cursor. Other MCP-capable hosts use the bundled `agent-room-mcp` binary through the desktop runtime's generated configuration; see [Configure another MCP host](./manual-mcp-hosts.md). They do not receive one-click configuration or vendor-specific acceptance coverage.

## Server platforms

| Platform                                      | Engineering status                                                             | Public support status                    |
| --------------------------------------------- | ------------------------------------------------------------------------------ | ---------------------------------------- |
| Dedicated Linux x86-64 + Docker Compose 2.20+ | Production reference, validation, backup, restore, and diagnostics implemented | Clean-host/public-DNS acceptance pending |
| Linux arm64 server                            | OCI multi-architecture build path exists                                       | Host-level production acceptance pending |
| Kubernetes                                    | Intentionally not implemented                                                  | Unsupported                              |
| Windows/macOS server host                     | Render/validation may run; installer rejects production use                    | Unsupported                              |

The default single-host profile uses PostgreSQL 18, Synapse 1.159, Keycloak 26.7, SeaweedFS 4.44, Redis 8.2 when workers are enabled, Caddy, ClamAV, OpenTelemetry Collector, Prometheus, Alertmanager, and Grafana at the image versions pinned in `infra/production/compose.yaml`.

## External services

- PostgreSQL must provide TLS `require`, `verify-ca`, or `verify-full`, the fixed least-privilege roles, and verifiable PITR meeting the configured RPO.
- S3-compatible storage must support bucket health checks and the object operations used by the content adapter; the bucket is pre-created for external mode.
- OIDC must satisfy the Authorization Code + PKCE and device authorization contracts implemented by the control plane and Bridge.
- Matrix federation compatibility is constrained by the pinned Synapse release and Agent Room event negotiation, not by arbitrary Matrix clients.

See [Self-hosting](./self-hosting.md) and [Known limitations](./known-limitations.md).
