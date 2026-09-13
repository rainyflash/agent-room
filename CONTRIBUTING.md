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

### Desktop development and packaging

Use the same entrypoints locally and in the Windows candidate workflow:

| Command                                 | Purpose                                                                                                                                    |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `corepack pnpm@10.28.0 desktop:dev`     | Build the UI and run the real desktop with saved login. No installer needed.                                                               |
| `corepack pnpm@10.28.0 desktop:preview` | Build the UI and run the real desktop against the default service, using the installed app's origin and saved login. No installer needed.  |
| `corepack pnpm@10.28.0 desktop:check`   | Run native adapter/Bridge/desktop checks, UI tests, and the agent access browser regressions.                                              |
| `corepack pnpm@10.28.0 desktop:hosts`   | Build the diagnostic entrypoint and inspect the real installed tools through the production adapters, without editing their configuration. |
| `corepack pnpm@10.28.0 desktop:package` | Run the checks, stop on failure, then build a local NSIS installer.                                                                        |

`node tools/desktop.mjs <mode> --plan` prints the commands without running them. Build concurrency defaults to four jobs to keep a development computer responsive; `CARGO_BUILD_JOBS` overrides it. The read-only host report distinguishes not installed, configuration needed, and a failed configuration read. A report with no installed hosts does not prove compatibility with a real host.

`desktop:dev` and `desktop:preview` are the same native entrypoint. It waits for the frontend build before compiling the native shell, uses `http://tauri.localhost`, and disables the development web server. This preserves the production cookie and CORS boundaries; an ordinary Vite page cannot substitute for it. The public desktop endpoints live in `apps/web/.env.desktop` and can be overridden with environment variables. Restart the preview after editing native or embedded UI code. For rapid layout work, use the existing `just web` hot reload with the local backend setup above; return to the native preview to verify desktop behavior.

Close an already-running desktop from its tray before switching to a development build. A preview shares the normal device authorization and account storage. Configuring a host from a preview points that host to the debug MCP binary; after upgrading the installed application, run its one-click configuration again to select the installed binary.

Desktop checks start their own browser test server on port 14174 with the test API settings. They never reuse a running development server. Set `AGENT_ROOM_E2E_PORT` to select another free port; a busy port fails instead of silently testing a different environment.

Local packages are for validation. Public releases continue through the signed candidate and promotion workflow in [the release runbook](./docs/operations/signed-releases.md). Use its `client` profile for desktop-only changes; use `full` when server images also change. Before promotion, test the actual installed host and the native invite dialog, including an already-authorized device with no default Agent and recovery from a connection failure. Browser fixtures validate UI states; they do not establish that a real Codex task has joined a production room.

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
