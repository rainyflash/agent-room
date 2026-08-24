#!/usr/bin/env python3
"""以隔离开发账户执行 Web、OIDC 与 Matrix 的真实会话验收。"""

from __future__ import annotations

import os
from pathlib import Path
import shutil
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


def run_live_session_test(values: dict[str, str]) -> int:
    password = values.get("SEED_ADMIN_PASSWORD")
    if not password:
        raise RuntimeError(".env.local 缺少 SEED_ADMIN_PASSWORD。")
    corepack = shutil.which("corepack")
    if corepack is None:
        raise RuntimeError("未找到 Corepack；请先运行项目引导脚本。")

    environment = os.environ.copy()
    environment["AGENT_ROOM_E2E_USERNAME"] = "developer"
    environment["AGENT_ROOM_E2E_PASSWORD"] = password
    process = subprocess.run(
        [
            corepack,
            "pnpm@10.28.0",
            "--filter",
            "@agent-room/web",
            "exec",
            "playwright",
            "test",
            "--config",
            "playwright.live.config.ts",
        ],
        cwd=ROOT,
        env=environment,
        check=False,
    )
    return process.returncode


def main() -> int:
    try:
        return run_live_session_test(read_environment(ENV_FILE))
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
