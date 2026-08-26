#!/usr/bin/env python3
"""生成、安装、升级和诊断 Agent Room 生产 Compose。"""

from __future__ import annotations

import argparse
from pathlib import Path
import sys

from prodops.config import DeploymentConfigError, load_deployment_config
from prodops.render import DeploymentPaths
from prodops.runtime import ProductionRuntime, ProductionRuntimeError
from prodops.secrets import SecretStoreError


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action",
        choices=("render", "validate", "preflight", "install", "upgrade", "health", "federation", "down"),
    )
    parser.add_argument("--config", type=Path, required=True, help="部署 JSON 配置")
    parser.add_argument("--state-dir", type=Path, required=True, help="持久状态与 Secret 目录")
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
            case "down":
                runtime.down()
            case _:
                raise AssertionError("argparse 已约束 action")
    except (DeploymentConfigError, ProductionRuntimeError, SecretStoreError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
