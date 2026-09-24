#!/usr/bin/env python3
"""生成、安装、升级和诊断 Agent Room 生产 Compose。"""

from __future__ import annotations

import argparse
from pathlib import Path
import sys

from prodops.backup import BackupError
from prodops.config import DeploymentConfigError, load_deployment_config
from prodops.render import DeploymentPaths
from prodops.runtime import ProductionRuntime, ProductionRuntimeError
from prodops.restore import RestoreDrillError
from prodops.schedule import BackupScheduleError, BackupScheduleInstaller
from prodops.secrets import SecretStoreError


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action",
        choices=(
            "render",
            "validate",
            "preflight",
            "install",
            "upgrade",
            "health",
            "federation",
            "backup",
            "backup-verify",
            "backup-prune",
            "backup-schedule-install",
            "backup-schedule-render",
            "backup-schedule-verify",
            "restore-drill",
            "network-agent-disable",
            "down",
        ),
    )
    parser.add_argument("--config", type=Path, required=True, help="部署 JSON 配置")
    parser.add_argument("--state-dir", type=Path, required=True, help="持久状态与 Secret 目录")
    parser.add_argument("--backup-id", help="要校验或恢复的备份 ID")
    parser.add_argument(
        "--network-agent",
        help="要停用的网络 Agent：网络 Agent ID、它的 Agent ID，或还在用的名字",
    )
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    try:
        config = load_deployment_config(arguments.config)
        paths = DeploymentPaths.from_state(arguments.state_dir)
        runtime = ProductionRuntime(config, paths)
        match arguments.action:
            case "render":
                runtime.prepare(generate_signing_key=False)
            case "validate":
                runtime.prepare(generate_signing_key=False)
                runtime.validate_compose()
            case "preflight":
                report = runtime.preflight(require_linux=True, require_dns=True, require_ports=True)
                for warning in report.warnings:
                    print(f"警告：{warning}")
            case "install":
                runtime.install()
            case "upgrade":
                runtime.upgrade()
            case "health":
                runtime.health(timeout_seconds=30)
            case "federation":
                runtime.federation(timeout_seconds=15)
            case "backup":
                manifest = runtime.backup()
                print(f"备份已原子发布：{manifest.backup_id}")
            case "backup-verify":
                if not arguments.backup_id:
                    raise ValueError("backup-verify 必须提供 --backup-id。")
                manifest = runtime.verify_backup(arguments.backup_id)
                print(f"备份完整性验证通过：{manifest.backup_id}")
            case "backup-prune":
                removed = runtime.prune_backups()
                print(f"已清理 {len(removed)} 个过期备份。")
            case "backup-schedule-render":
                installer = BackupScheduleInstaller(config, paths, arguments.config)
                service, timer = installer.write_generated()
                print(f"备份调度已生成：{service}；{timer}")
            case "backup-schedule-install":
                runtime.prepare(generate_signing_key=True)
                runtime.validate_compose()
                files = BackupScheduleInstaller(config, paths, arguments.config).install()
                print(f"备份调度已启用：{files.timer_name}")
            case "backup-schedule-verify":
                installer = BackupScheduleInstaller(config, paths, arguments.config)
                installer.verify(installer.render())
                print("备份调度已启用且正在运行。")
            case "restore-drill":
                if not arguments.backup_id:
                    raise ValueError("restore-drill 必须提供 --backup-id。")
                report = runtime.restore_drill(arguments.backup_id)
                print(
                    f"隔离恢复演练通过：{report.backup_id}，"
                    f"耗时 {report.duration_seconds:.3f} 秒。"
                )
            case "network-agent-disable":
                if not arguments.network_agent:
                    raise ValueError("network-agent-disable 必须提供 --network-agent。")
                disabled = runtime.disable_network_agent(arguments.network_agent)
                if not disabled:
                    raise ValueError("没有找到还在用的这个网络 Agent。")
                for line in disabled:
                    print(f"已停用网络 Agent：{line}。令牌已作废，约一分钟内离开所有房间。")
            case "down":
                runtime.down()
            case _:
                raise AssertionError("argparse 已约束 action")
    except (
        BackupError,
        BackupScheduleError,
        DeploymentConfigError,
        ProductionRuntimeError,
        RestoreDrillError,
        SecretStoreError,
        ValueError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
