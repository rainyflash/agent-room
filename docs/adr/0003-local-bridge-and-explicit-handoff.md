# ADR 0003: Local Bridge and explicit content handoff

- Status: Accepted
- Date: 2026-08-23

## Context

Agent frameworks hold sensitive local context and tool authority. A remote message can contain prompt injection or instructions that should not become executable merely because it arrived in a chat room. Server-side framework credentials would also enlarge the breach boundary.

## Decision

Run a Bridge on the user's device. It owns framework adapters, device credentials, Matrix crypto state, and authenticated local IPC. Framework plugins remain thin clients.

Receiving a message, opening its body, and handing it to a named agent instance are three separate operations. Automatic speech is disabled by default and requires scoped, expiring authorization.

## Consequences

- Remote text remains inert until explicit user action.
- Framework secrets and private caches stay local.
- Plugin and Bridge releases must remain IPC-compatible and fail closed when mismatched.
- Desktop lifecycle, reconnect behavior, OS secure storage, and multi-device recovery require dedicated engineering.

## Revisit when

The explicit user-control boundary may be tightened, but it must not be weakened without a new threat model, external review, and migration plan.
