set dotenv-load := true
set shell := ["bash", "-euo", "pipefail", "-c"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]
pnpm := "corepack pnpm@10.28.0"

default:
  @just --list

bootstrap:
  node tools/bootstrap.mjs

format:
  cargo fmt --all
  {{pnpm}} format

format-check:
  cargo fmt --all --check
  {{pnpm}} format:check

lint:
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  {{pnpm}} lint

typecheck:
  cargo check --workspace --all-targets --all-features
  {{pnpm}} typecheck

build:
  {{pnpm}} build

test:
  cargo test --workspace --all-features
  {{pnpm}} test

python-test:
  python -m unittest discover -s tools/tests -p "test_*.py"

coverage:
  python tools/database.py coverage
  {{pnpm}} test:coverage

protocol-generate:
  {{pnpm}} protocol:generate

protocol-check:
  {{pnpm}} protocol:check
  cargo test -p agent-room-protocol-conformance

check: format-check lint typecheck build test python-test protocol-check
  {{pnpm}} secrets:check
  {{pnpm}} actions:check

dev-up:
  node tools/run-powershell.mjs tools/dev-infra.ps1 up

dev-down:
  node tools/run-powershell.mjs tools/dev-infra.ps1 down

dev-reset:
  node tools/run-powershell.mjs tools/dev-infra.ps1 reset

dev-seed:
  node tools/run-powershell.mjs tools/dev-infra.ps1 seed

health:
  node tools/run-powershell.mjs tools/dev-infra.ps1 health

database-migrate:
  python tools/database.py migrate

database-integration:
  python tools/database.py test

object-store-integration:
  python tools/object_store.py

content-integration:
  python tools/content.py

reliability:
  python tools/reliability.py

security:
  python tools/security.py

control-plane: database-migrate
  python tools/control-plane.py run

web:
  {{pnpm}} --filter @agent-room/web dev

web-browser:
  {{pnpm}} --filter @agent-room/web test:browser

web-session-integration: database-migrate
  python tools/web.py

vertical-bootstrap:
  python tools/vertical.py bootstrap

security-vertical:
  python tools/vertical.py security

closed-test-matrix:
  python tools/closed_test.py matrix

closed-test-package:
  python tools/closed_test.py package

closed-test-verify:
  python tools/closed_test.py verify --required-platform windows-x64 --required-platform macos-arm64

federation:
  python tools/federation.py bootstrap

federation-diagnose:
  python tools/federation.py diagnose

control-plane-integration: database-migrate
  python tools/control-plane.py test

matrix-integration:
  python tools/matrix.py

bridge:
  python tools/bridge.py

plugin-validate:
  python tools/plugin.py validate

plugin-package:
  python tools/plugin.py stage

plugin-host-check:
  python tools/plugin.py host-check

infra-config:
  node tools/run-powershell.mjs tools/dev-infra.ps1 config

sbom:
  node tools/run-powershell.mjs tools/sbom.ps1
