# Contributing to Agent Room

Agent Room accepts focused, evidence-backed contributions. This repository crosses identity, E2EE, federation, local process security, and durable data boundaries; casual patches in those areas create real risk.

## Before opening a change

1. Read the [requirements](./specs/agent-room-foundation/requirements.md), [technical design](./specs/agent-room-foundation/design.md), and [implementation plan](./specs/agent-room-foundation/tasks.md).
2. Search existing issues and pull requests.
3. Open an issue before a protocol break, persistence migration, trust-boundary change, or new infrastructure dependency.
4. Report vulnerabilities privately under [SECURITY.md](./SECURITY.md), never through an issue or pull request.

## Development setup

Install Git 2.40+, Node.js 24, Rust through rustup, Docker Engine with Compose 2.20+, and Python 3.11+. Then run:

```bash
node tools/bootstrap.mjs
just dev-up
just database-migrate
just dev-seed
```

The bootstrap validates tool versions, installs the locked Node dependencies, regenerates protocol bindings, and fetches locked Rust dependencies. `node tools/bootstrap.mjs --check` or `just doctor` performs the checks without changing the workspace.

Run `just control-plane` and `just web` in separate terminals. Use `just dev-down` when finished.

## Architecture rules

- Dependencies point inward: UI and adapters depend on application ports and domain types, never the reverse.
- Organize product work by feature and keep rendering, state glue, use cases, and external services separate.
- Domain logic belongs in pure Rust or TypeScript modules, not React components, HTTP handlers, or database adapters.
- External systems are injected behind interfaces and must be mockable.
- Cross-language payloads start in `packages/protocol/schema`; regenerate types instead of editing generated output.
- Avoid unbounded inputs, silent fallbacks, swallowed errors, and identity-bearing metric labels.
- Remote content must remain inert until the user explicitly opens or hands it off.

Read [Architecture](./docs/architecture.md) and the [ADRs](./docs/adr/README.md) before introducing a new dependency direction.

## Tests and generated files

Every behavior change needs a success case, a boundary case, and a failure case at the narrowest useful layer. Before requesting review, run:

```bash
just check
```

Also run the relevant integration command when touching Matrix, PostgreSQL, object storage, federation, production operations, or browser behavior. Do not hand-edit generated protocol types, release inventories, SBOMs, license inventories, or files under `.local/`.

## Commits and pull requests

- Keep each commit independently explainable and use an imperative Conventional Commit subject.
- Link the relevant task and requirement when the change is part of the tracked plan.
- Include migration and rollback behavior for schema or protocol changes.
- Include exact commands and results used for verification.
- Update English and Chinese user-facing text together when behavior changes.
- Never commit real credentials, private messages, local workspace paths, production endpoints, or generated secrets.

Reviewers will reject a patch that bypasses an existing abstraction, duplicates a source of truth, weakens fail-closed behavior, or claims acceptance without evidence. That is product safety, not ceremony.
