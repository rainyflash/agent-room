#!/usr/bin/env python3
"""为本地控制平面注入脱敏配置并执行运行或真实依赖测试。"""

from __future__ import annotations

import argparse
import subprocess
import sys

from local_runtime import (
    ROOT,
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
    arguments = parser.parse_args()

    try:
        values = read_environment()
        environment = control_plane_runtime_environment(
            values, enable_telemetry=arguments.action == "run"
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
