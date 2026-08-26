set dotenv-load := true
set shell := ["bash", "-euo", "pipefail", "-c"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]
pnpm := "corepack pnpm@10.28.0"

default:
  @just --list

bootstrap:
  node tools/bootstrap.mjs

doctor:
  node tools/bootstrap.mjs --check

licenses:
  python tools/license_inventory.py generate

licenses-check:
  python tools/license_inventory.py check

oss-check:
  python tools/open_source.py
  python tools/license_inventory.py check

oss-acceptance:
  python tools/open_source_acceptance.py

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
  python tools/license_inventory.py check
  python tools/open_source.py

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

production-render config state:
  python tools/production.py render --config {{config}} --state-dir {{state}}

production-validate config state:
  python tools/production.py validate --config {{config}} --state-dir {{state}}

production-install config state:
  python tools/production.py install --config {{config}} --state-dir {{state}}

production-upgrade config state:
  python tools/production.py upgrade --config {{config}} --state-dir {{state}}

production-health config state:
  python tools/production.py health --config {{config}} --state-dir {{state}}

self-host-init domain email output:
  python tools/self_host.py init --domain {{domain}} --email {{email}} --output {{output}}

self-host-doctor config state:
  python tools/self_host.py doctor --config {{config}} --state-dir {{state}}

self-host-install config state:
  python tools/self_host.py install --config {{config}} --state-dir {{state}}

self-host-upgrade config state:
  python tools/self_host.py upgrade --config {{config}} --state-dir {{state}}

observability-validate:
  python tools/observability.py validate

observability-drill config state:
  python tools/observability.py drill --config {{config}} --state-dir {{state}} --target all --confirm-stop-services
