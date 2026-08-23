#!/usr/bin/env python3
"""为本地控制平面注入脱敏配置并执行运行或真实依赖测试。"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"


def read_environment(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise RuntimeError("缺少 .env.local；请先运行 just dev-up。")

    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key:
            raise RuntimeError(".env.local 包含无效配置行。")
        values[key] = value
    return values


def runtime_environment(values: dict[str, str], enable_telemetry: bool) -> dict[str, str]:
    database_password = values.get("AGENT_ROOM_DB_PASSWORD")
    if not database_password:
        raise RuntimeError(".env.local 缺少 AGENT_ROOM_DB_PASSWORD。")

    environment = os.environ.copy()
    environment.update(
        {
            "AGENT_ROOM_BIND_ADDRESS": "127.0.0.1:3000",
            "AGENT_ROOM_DB_HOST": "127.0.0.1",
            "AGENT_ROOM_DB_PORT": "55432",
            "AGENT_ROOM_DB_NAME": "agent_room",
            "AGENT_ROOM_DB_USER": "agent_room",
            "AGENT_ROOM_DB_PASSWORD": database_password,
            "AGENT_ROOM_DB_TLS_MODE": "disable",
            "AGENT_ROOM_MATRIX_BASE_URL": "http://127.0.0.1:18008",
            "AGENT_ROOM_OBJECT_STORE_HEALTH_URL": (
                "http://127.0.0.1:19333/cluster/status"
            ),
            "AGENT_ROOM_DEPENDENCY_TIMEOUT_MS": "2000",
            "AGENT_ROOM_OTEL_EXPORT_TIMEOUT_MS": "5000",
            "AGENT_ROOM_LOG_FILTER": (
                "agent_room_control_plane=info,sqlx=warn"
            ),
        }
    )
    if enable_telemetry:
        environment["AGENT_ROOM_OTLP_TRACES_ENDPOINT"] = (
            "http://127.0.0.1:14318/v1/traces"
        )
    else:
        environment.pop("AGENT_ROOM_OTLP_TRACES_ENDPOINT", None)
    return environment


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
        values = read_environment(ENV_FILE)
        environment = runtime_environment(values, enable_telemetry=arguments.action == "run")
    except RuntimeError as error:
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
