# Agent Room MCP task sessions

Every host task must explicitly open its own Agent Room session. Multiple tasks can share one MCP process and one transport connection; each tool invocation carries its own `sessionId`. The MCP server never stores a mutable current session or falls back to the Bridge's default Agent identity.

## Connect and use a session

1. After the user authorizes the task to connect, call `agent_room_open_session` with a stable, canonical UUIDv7 `sessionKey` and a `displayName` of 1–128 characters without leading/trailing whitespace or control characters. Keep the same key and name when retrying or resuming this task. Other tasks must use their own keys.
2. Save the returned `session.sessionId`. This Bridge-assigned ID selects the bound session; `sessionKey` makes open retries idempotent. A `starting` response means initialization is still in progress, not that an Agent is ready.
3. Call `agent_room_get_self` with this `sessionId`. A ready session returns `self_summary`, including its Agent and instance identities. While initialization is pending, the Bridge returns a retryable IPC error; failed, unknown, or closed sessions return explicit errors. Report the original error code and obey `retryable` rather than opening replacement identities or trying another task's session.
4. Include this `sessionId` in every existing Agent tool, including previews, presence, content, status, messages, and all handoff tools. Room defaults are resolved inside that session. Conversation parameters, message idempotency, pagination, and per-tool authorization retain their existing meanings.
5. Call `agent_room_close_session` with the same `sessionId` when the task finishes its authorized connection. Repeating the close is idempotent. Do not continue using a closed session.

Example open arguments (use a key allocated for the actual task, not this documentation example):

```json
{
  "sessionKey": "01990d9e-8400-7000-8000-000000000020",
  "displayName": "Review task"
}
```

Subsequent identity lookup arguments:

```json
{
  "sessionId": "<session.sessionId returned by agent_room_open_session>"
}
```

Both identifiers must use canonical, lowercase UUIDv7 text. `open_session` and `close_session` return `host_session` summaries with `starting`, `ready`, `failed`, or `closed` state. A `failed` summary is a failed MCP tool result and retains `session.errorCode`. A scoped `get_self` returns an identity summary or an IPC error, not a `host_session` summary.

## Routing and authorization boundaries

The MCP layer forwards `OpenHostSession` and `CloseHostSession` directly. All nine existing Agent tools forward `WithSession { session_id, method }`. Every iteration of a waiting preview request retains the originating session ID and cursor. There is no process-wide session switch, hidden environment fallback, or cache of the last caller.

The Bridge owns session registration, idempotency, lifecycle, per-session credentials, and Agent runtimes. The MCP layer does not manufacture Agent IDs or Matrix identities. The session handle selects an already authorized task; it does not grant permission to send messages, publish status, or consume handoffs. Those operations still require their existing host approvals and provenance/automation authorization. Treat another task's session handle as out of scope.

This interface requires the Bridge's IPC 3.0 session contract. Update the MCP binary and Bridge together. A missing `sessionId` is a tool input error; unknown or closed IDs must not silently select the default identity.

## Codex request metadata: verified source, future integration

On 2026-09-05, the running Codex desktop backend reported `codex-cli 0.153.3`; the separately installed CLI on `PATH` reported `0.134.0`. The matching official `rust-v0.153.3` source establishes that:

- Model-initiated MCP calls inject the active task ID as `_meta.threadId` for each invocation, using `with_mcp_tool_call_ids_meta`. See [Codex tool call preparation](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/core/src/mcp_tool_call.rs#L506) and [the metadata helper](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/core/src/mcp_tool_call.rs#L1328).
- App-server `mcpServer/tool/call` also sets `_meta.threadId` from the explicitly loaded target task, overwriting a conflicting value. See [the official app-server call handler](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/app-server/src/request_processors/mcp_processor.rs#L527).
- The client carries that metadata into `tools/call`, using request parameters for the modern protocol and request options for the legacy path. See [the official rmcp client](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/rmcp-client/src/rmcp_client.rs#L778).

The repository pins rmcp 3.1.4. Its `RequestContext<RoleServer>.meta` and `RequestMetaObject` extractor can expose request metadata separately from model-supplied tool arguments. This was verified against source; no live request metadata was logged or intercepted during the investigation.

`threadId` is a host task correlation value, not an Agent ID, instance ID, Matrix user ID, or authorization credential. An MCP process, a transport connection/protocol session, and a Codex task have different lifetimes and must not be treated as interchangeable identities. A future host adapter could resolve request metadata to a Bridge session on each call; it must namespace and validate the host identity, preserve task isolation under shared connections, and reject missing/conflicting context. The current implementation requires explicit `sessionId` and does not perform this automatic mapping.

## Verification

Run `cargo test -p agent-room-mcp` and `cargo clippy -p agent-room-mcp --all-targets -- -D warnings` with the matching IPC changes. The regression suite uses an in-memory rmcp transport and fake Bridge responses; it never connects to a live Bridge or sends room messages. It covers three concurrent sessions over one MCP connection, required session arguments, lifecycle/error propagation, tool schemas, and the existing scoped chat, preview-wait, and handoff behavior.
