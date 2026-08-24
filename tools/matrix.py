#!/usr/bin/env python3
"""为真实 Matrix 适配器验收注入脱敏配置并运行测试。"""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"


def read_environment(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise RuntimeError("缺少 .env.local；请先运行 just dev-up 和 just dev-seed。")

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


def runtime_environment(values: dict[str, str]) -> dict[str, str]:
    required = {
        "SEED_ADMIN_PASSWORD": values.get("SEED_ADMIN_PASSWORD"),
        "SEED_AGENT_PASSWORD": values.get("SEED_AGENT_PASSWORD"),
    }
    missing = [name for name, value in required.items() if not value]
    if missing:
        raise RuntimeError(f".env.local 缺少 {', '.join(missing)}。")

    environment = os.environ.copy()
    environment.update(
        {
            "AGENT_ROOM_MATRIX_TEST_BASE_URL": "http://127.0.0.1:18008",
            "AGENT_ROOM_MATRIX_TEST_ADMIN_USER": (
                "@developer:matrix.agent-room.localhost"
            ),
            "AGENT_ROOM_MATRIX_TEST_ADMIN_PASSWORD": required[
                "SEED_ADMIN_PASSWORD"
            ],
            "AGENT_ROOM_MATRIX_TEST_AGENT_USER": (
                "@agent-alpha:matrix.agent-room.localhost"
            ),
            "AGENT_ROOM_MATRIX_TEST_AGENT_PASSWORD": required[
                "SEED_AGENT_PASSWORD"
            ],
        }
    )
    return environment


def main() -> int:
    try:
        environment = runtime_environment(read_environment(ENV_FILE))
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 2

    command = [
        "cargo",
        "test",
        "-p",
        "agent-room-matrix-adapter",
        "--test",
        "real_synapse",
        "--",
        "--ignored",
        "--test-threads=1",
    ]
    return subprocess.run(command, cwd=ROOT, env=environment, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
