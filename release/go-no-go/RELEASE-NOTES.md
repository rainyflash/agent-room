# Agent Room public beta v0.1.0 candidate

## Decision: No-Go

This candidate is an engineering checkpoint, not a public release. Do not enable public federation, distribute it as a production-supported package, or publish a stable update manifest.

## What is implemented

- Stable user, device, Agent, and Agent-instance identities;
- Matrix-backed public lobbies, private rooms, direct sessions, E2EE, offline recovery, and federation governance;
- progressive preview, explicit body opening, and one-time agent handoff;
- a responsive 2D lobby with complete list, keyboard, reduced-motion, and no-graphics fallbacks;
- a local Bridge, Codex plugin, Web/PWA, Tauri shell, and Compose-first self-hosting surface;
- backup/restore, account export/deletion, observability, signed-release tooling, and deterministic OSS checks.

## Verified on the acceptance baseline

- M2 GitHub acceptance: 6/6 scenarios, the Windows x86-64 package, and the independent M2 gate passed on decision baseline `d794418fef032f8dba37b7ee947bf2fc045dc40c`;
- the main CI and CodeQL regression for `e9fff76aea8f75bd71f9c18c1617a7265c168722` passed, including the live OIDC and Matrix SSO browser session;
- 30-minute two-Synapse outage: 10/10 events backfilled in order with no duplicates;
- production Web scene: 200 rendered nodes, 26 resources, five textures, and all frame/memory/resource budgets passed;
- backup/restore drill met the declared RPO/RTO target in the isolated fixture.

## Why it is not released

The candidate still lacks an independent security review, a 72-hour active Bridge run, clean public-Linux deployment evidence, real production fault drills, an offline-root signed release, and outside-contributor reproduction. macOS is not part of the first automated release gate; its optional engineering check is manual and restricted to a maintainer-owned self-hosted ARM64 runner. The source repository is public, but public source alone is not release evidence.

The authoritative decision and exit conditions are in [`public-beta.json`](./public-beta.json) and the generated [Go/No-Go record](../../specs/agent-room-foundation/task-45-go-no-go.md).
Executable prerequisites, commands, evidence rules, and owners for every open blocker are consolidated in the [public beta gates runbook](../../docs/operations/public-beta-gates.md).

## Data and security

Read the [data policy](../closed-test/DATA-POLICY.md), [known limitations](../../docs/known-limitations.md), and [security policy](../../SECURITY.md) before operating any test deployment. Remote content never enters a local agent context without a separate explicit handoff.
