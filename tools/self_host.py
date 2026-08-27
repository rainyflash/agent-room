#!/usr/bin/env python3
"""生成配置并运行 Agent Room 自托管生命周期。"""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import sys
from typing import Final

from prodops.self_host import SelfHostConfig, SelfHostConfigError, write_new_config


ROOT: Final = Path(__file__).resolve().parents[1]
PRODUCTION_TOOL: Final = ROOT / "tools" / "production.py"
ACTION_MAP: Final = {
    "doctor": "preflight",
    "install": "install",
    "upgrade": "upgrade",
    "health": "health",
    "federation": "federation",
    "backup": "backup",
    "backup-verify": "backup-verify",
    "backup-prune": "backup-prune",
    "backup-schedule-install": "backup-schedule-install",
    "backup-schedule-verify": "backup-schedule-verify",
    "restore-drill": "restore-drill",
    "stop": "down",
}


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    initialize = subcommands.add_parser("init", help="生成经过严格验证的生产配置")
    initialize.add_argument("--domain", required=True, help="公开基础域名，例如 room.example.com")
    initialize.add_argument(
        "--email",
        help="可选的 ACME 联系邮箱，仅用于证书机构通知",
    )
    initialize.add_argument("--output", type=Path, required=True, help="要新建的部署 JSON")
    initialize.add_argument("--project-name", default="agent-room")
    initialize.add_argument("--backup-repository", default="/var/backups/agent-room")
    initialize.add_argument("--retention-days", type=int, default=30)
    initialize.add_argument("--rpo-minutes", type=int, default=15)
    initialize.add_argument("--database-mode", choices=("embedded", "external"), default="embedded")
    initialize.add_argument("--database-host")
    initialize.add_argument("--database-port", type=int, default=5432)
    initialize.add_argument(
        "--database-tls-mode",
        choices=("disable", "prefer", "require", "verify-ca", "verify-full"),
    )
    initialize.add_argument("--provider-pitr-evidence-file")
    initialize.add_argument(
        "--object-store-mode", choices=("embedded", "external"), default="embedded"
    )
    initialize.add_argument("--object-store-endpoint")
    initialize.add_argument("--object-store-health-url")
    initialize.add_argument("--object-store-bucket", default="agent-room-content")
    initialize.add_argument("--object-store-region", default="us-east-1")
    initialize.add_argument("--control-plane-replicas", type=int, default=1)
    initialize.add_argument("--synapse-workers", type=int, default=0)
    initialize.add_argument(
        "--alert-webhook-url",
        help="提供时启用完整遥测；URL 中不得包含凭据",
    )

    for command in ACTION_MAP:
        lifecycle = subcommands.add_parser(command)
        lifecycle.add_argument("--config", type=Path, required=True)
        lifecycle.add_argument("--state-dir", type=Path, required=True)
        if command in {"backup-verify", "restore-drill"}:
            lifecycle.add_argument("--backup-id", required=True)
    return parser


def create_config(arguments: argparse.Namespace) -> int:
    config = SelfHostConfig(
        domain=arguments.domain,
        acme_email=arguments.email,
        project_name=arguments.project_name,
        backup_repository=arguments.backup_repository,
        retention_days=arguments.retention_days,
        rpo_minutes=arguments.rpo_minutes,
        database_mode=arguments.database_mode,
        database_host=arguments.database_host,
        database_port=arguments.database_port,
        database_tls_mode=arguments.database_tls_mode,
        provider_pitr_evidence_file=arguments.provider_pitr_evidence_file,
        object_store_mode=arguments.object_store_mode,
        object_store_endpoint=arguments.object_store_endpoint,
        object_store_health_url=arguments.object_store_health_url,
        object_store_bucket=arguments.object_store_bucket,
        object_store_region=arguments.object_store_region,
        control_plane_replicas=arguments.control_plane_replicas,
        synapse_workers=arguments.synapse_workers,
        alert_webhook_url=arguments.alert_webhook_url,
    )
    try:
        write_new_config(config, arguments.output)
    except SelfHostConfigError as error:
        print(str(error), file=sys.stderr)
        return 1
    print(f"配置已生成并通过领域校验：{arguments.output}")
    return 0


def run_lifecycle(arguments: argparse.Namespace) -> int:
    action = ACTION_MAP[arguments.command]
    command = [
        sys.executable,
        str(PRODUCTION_TOOL),
        action,
        "--config",
        str(arguments.config),
        "--state-dir",
        str(arguments.state_dir),
    ]
    backup_id = getattr(arguments, "backup_id", None)
    if backup_id is not None:
        command.extend(("--backup-id", backup_id))
    return subprocess.run(command, cwd=ROOT, check=False).returncode


def main() -> int:
    arguments = build_parser().parse_args()
    try:
        if arguments.command == "init":
            return create_config(arguments)
        return run_lifecycle(arguments)
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
