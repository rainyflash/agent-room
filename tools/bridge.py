#!/usr/bin/env python3
"""为本地 Bridge 注入非敏感开发配置并启动设备授权流程。"""

from __future__ import annotations

from pathlib import Path
import subprocess
import sys
from typing import Final

if __package__:
    from .local_runtime import LocalRuntimeError, bridge_runtime_environment
else:
    from local_runtime import LocalRuntimeError, bridge_runtime_environment


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"


def runtime_environment() -> dict[str, str]:
    if not ENV_FILE.is_file():
        raise LocalRuntimeError("缺少 .env.local；请先运行 just dev-up。")
    return bridge_runtime_environment(
        data_root=(ROOT / ".local" / "bridge").resolve()
    )


def main() -> int:
    try:
        environment = runtime_environment()
    except LocalRuntimeError as error:
        print(str(error), file=sys.stderr)
        return 2

    completed = subprocess.run(
        ["cargo", "run", "-p", "agent-room-bridge"],
        cwd=ROOT,
        env=environment,
        check=False,
    )
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
