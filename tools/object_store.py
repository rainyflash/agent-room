#!/usr/bin/env python3
"""准备本地私有对象桶并运行真实 SeaweedFS 兼容性测试。"""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import sys
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"
SAFE_SECRET: Final = re.compile(r"^[A-Za-z0-9._~-]{3,256}$")


def read_environment() -> dict[str, str]:
    if not ENV_FILE.is_file():
        raise RuntimeError("缺少 .env.local；请先运行 just dev-up。")

    values: dict[str, str] = {}
    for raw_line in ENV_FILE.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key:
            raise RuntimeError(".env.local 包含无效配置行。")
        values[key] = value
    return values


def required_value(values: dict[str, str], name: str) -> str:
    value = values.get(name)
    if value is None or SAFE_SECRET.fullmatch(value) is None:
        raise RuntimeError(f".env.local 中的 {name} 缺失或格式非法。")
    return value


def run(command: list[str], environment: dict[str, str]) -> None:
    result = subprocess.run(command, cwd=ROOT, env=environment, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"命令执行失败，退出码为 {result.returncode}。")


def main() -> int:
    try:
        values = read_environment()
        environment = os.environ.copy()
        environment.update(
            {
                "AGENT_ROOM_TEST_S3_ENDPOINT": "http://127.0.0.1:18333",
                "AGENT_ROOM_TEST_S3_BUCKET": "agent-room-content",
                "AGENT_ROOM_TEST_S3_REGION": "us-east-1",
                "AGENT_ROOM_TEST_S3_ACCESS_KEY": required_value(
                    values, "S3_ACCESS_KEY"
                ),
                "AGENT_ROOM_TEST_S3_SECRET_KEY": required_value(
                    values, "S3_SECRET_KEY"
                ),
            }
        )
        # 复用唯一的基础设施种子入口，避免测试脚本复制桶创建和凭据传递逻辑。
        run(
            [
                "node",
                "tools/run-powershell.mjs",
                "tools/dev-infra.ps1",
                "seed",
            ],
            environment,
        )
        run(
            [
                "cargo",
                "test",
                "-p",
                "agent-room-content-adapter",
                "--test",
                "seaweedfs",
                "--",
                "--ignored",
                "--test-threads=1",
            ],
            environment,
        )
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
