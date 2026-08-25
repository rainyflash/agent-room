#!/usr/bin/env python3
"""构建真实 Bridge，并在隔离数据目录执行可恢复的 72 小时常驻观测。"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Final

if __package__:
    from .capacity import SOAK_SECONDS, CapacityFailure, run_bridge_soak
    from .local_runtime import LocalRuntimeError, bridge_runtime_environment
else:
    from capacity import SOAK_SECONDS, CapacityFailure, run_bridge_soak
    from local_runtime import LocalRuntimeError, bridge_runtime_environment


ROOT: Final = Path(__file__).resolve().parent.parent
DEFAULT_DATA_ROOT: Final = ROOT / ".local" / "capacity-bridge"


def bridge_binary() -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return ROOT / "target" / "release" / f"agent-room-bridge{suffix}"


def build_release_bridge() -> None:
    completed = subprocess.run(
        ["cargo", "build", "-p", "agent-room-bridge", "--release", "--locked"],
        cwd=ROOT,
        check=False,
    )
    if completed.returncode != 0:
        raise CapacityFailure("Release Bridge 构建失败。")


def runtime_environment(
    data_root: Path,
    agent_id: str | None,
    catalog_id: str | None,
) -> dict[str, str]:
    if (agent_id is None) != (catalog_id is None):
        raise CapacityFailure("Agent ID 与公共大厅目录 ID 必须同时提供。")
    return bridge_runtime_environment(
        data_root=data_root.resolve(),
        agent_id=agent_id,
        public_lobby_catalog_id=catalog_id,
        secure_storage_service="agent-room-capacity-bridge-v1",
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duration-seconds", type=int, default=SOAK_SECONDS)
    parser.add_argument("--sample-seconds", type=int, default=30)
    parser.add_argument("--agent-id")
    parser.add_argument("--catalog-id")
    parser.add_argument("--data-root", type=Path, default=DEFAULT_DATA_ROOT)
    parser.add_argument("--skip-build", action="store_true")
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    try:
        if not arguments.skip_build:
            build_release_bridge()
        executable = bridge_binary()
        environment = runtime_environment(
            arguments.data_root,
            arguments.agent_id,
            arguments.catalog_id,
        )
        report = run_bridge_soak(
            [str(executable)],
            arguments.duration_seconds,
            arguments.sample_seconds,
            environment=environment,
        )
        print(json.dumps(report["metrics"], ensure_ascii=False, indent=2))
        return 0 if report["passed"] is True else 1
    except (CapacityFailure, LocalRuntimeError, OSError) as error:
        print(f"Bridge 容量测试失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
