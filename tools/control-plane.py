#!/usr/bin/env python3
"""为本地控制平面注入脱敏配置并执行运行或真实依赖测试。"""

from __future__ import annotations

import argparse
import subprocess
import sys

from local_runtime import (
    ROOT,
    ControlPlaneNetworkScope,
    LocalRuntimeError,
    control_plane_runtime_environment,
    read_environment,
)


def command_for(action: str) -> list[str]:
    if action == "run":
        return ["cargo", "run", "-p", "agent-room-control-plane"]
    return [
        "cargo",
        "test",
        "-p",
        "agent-room-control-plane",
        "real_dependency_tests::",
        "--",
        "--ignored",
        "--test-threads=1",
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("run", "test"))
    parser.add_argument(
        "--network-scope",
        choices=tuple(ControlPlaneNetworkScope),
        default=ControlPlaneNetworkScope.LOOPBACK,
        type=ControlPlaneNetworkScope,
        help="控制平面监听边界；仅本地 TLS 网关验收需要 docker-gateway。",
    )
    arguments = parser.parse_args()

    if (
        arguments.action != "run"
        and arguments.network_scope != ControlPlaneNetworkScope.LOOPBACK
    ):
        parser.error("只有 run 操作可以扩大本地监听边界。")

    try:
        values = read_environment()
        environment = control_plane_runtime_environment(
            values,
            enable_telemetry=arguments.action == "run",
            network_scope=arguments.network_scope,
        )
    except LocalRuntimeError as error:
        print(str(error), file=sys.stderr)
        return 2

    process = subprocess.Popen(
        command_for(arguments.action),
        cwd=ROOT,
        env=environment,
    )
    try:
        return process.wait()
    except KeyboardInterrupt:
        try:
            return process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.terminate()
            return 130


if __name__ == "__main__":
    raise SystemExit(main())
