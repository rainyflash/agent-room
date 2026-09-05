# Agent Room Codex configuration adapter

This directory is the publishable Codex adapter template. A release archive places the same native `agent-room-mcp` binary used by Claude Code, Cursor, and other MCP hosts under `bin/agent-room-mcp`. Compiled binaries are not committed to the source repository.

The bundle contributes a skill and a local STDIO MCP definition. The single-instance Agent Room Bridge owns identity, Matrix sessions, device keys, and synchronization state.

## Task session lifecycle

After the user authorizes a task to connect, call `agent_room_open_session` with that task's stable, canonical UUIDv7 `sessionKey` and `displayName`. Save the Bridge-assigned `sessionId` and include it in every subsequent Agent tool, including `agent_room_get_self`. Retry an open with the same key and name; another task must use its own key and session. A `starting` response means initialization is pending, so query `get_self` with the returned session ID until the Bridge is ready or reports a non-retryable failure. Close the session with `agent_room_close_session` when finished.

Sharing an MCP process does not share a current session: every call is explicitly routed. Missing, unknown, or closed sessions never fall back to the default Agent identity. The plugin and Bridge must both support IPC 3.0. Task session IDs do not replace per-tool authorization or room automation grants.

## Approval model

Per-tool Codex approval is user configuration. The adapter neither owns nor silently rewrites it. `approval-policy.example.toml` shows a conservative baseline: identity, previews, and presence may be read directly; listing handoff metadata, opening full content, publishing status, sending messages, and consuming or declining handoffs ask each time. Replace only the plugin selector when the installed marketplace name differs.

## Local verification

- `just plugin-validate` validates the plugin structure, version, MCP definition, and approval policy.
- `just plugin-package` builds the native MCP for the current platform, runs a protocol smoke test, and creates a reproducible ZIP.
- `just plugin-host-check` installs into an isolated `CODEX_HOME` and verifies discovery from two independent Codex processes.
