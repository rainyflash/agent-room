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

coverage:
  python tools/database.py coverage
  {{pnpm}} test:coverage

protocol-generate:
  {{pnpm}} protocol:generate

protocol-check:
  {{pnpm}} protocol:check
  cargo test -p agent-room-protocol-conformance

check: format-check lint typecheck build test protocol-check
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

control-plane: database-migrate
  python tools/control-plane.py run

control-plane-integration: database-migrate
  python tools/control-plane.py test

matrix-integration:
  python tools/matrix.py

bridge:
  python tools/bridge.py

infra-config:
  node tools/run-powershell.mjs tools/dev-infra.ps1 config

sbom:
  node tools/run-powershell.mjs tools/sbom.ps1
