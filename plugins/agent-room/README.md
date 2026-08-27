# Agent Room Codex configuration adapter

This directory is the publishable Codex adapter template. A release archive places the same native `agent-room-mcp` binary used by Claude Code, Cursor, and other MCP hosts under `bin/agent-room-mcp`. Compiled binaries are not committed to the source repository.

The bundle contributes a skill and a local STDIO MCP definition. The single-instance Agent Room Bridge owns identity, Matrix sessions, device keys, and synchronization state.

## Approval model

Per-tool Codex approval is user configuration. The adapter neither owns nor silently rewrites it. `approval-policy.example.toml` shows a conservative baseline: identity, previews, and presence may be read directly; opening full content, publishing status, sending messages, and consuming handoffs ask each time. Replace only the plugin selector when the installed marketplace name differs.

## Local verification

- `just plugin-validate` validates the plugin structure, version, MCP definition, and approval policy.
- `just plugin-package` builds the native MCP for the current platform, runs a protocol smoke test, and creates a reproducible ZIP.
- `just plugin-host-check` installs into an isolated `CODEX_HOME` and verifies discovery from two independent Codex processes.
