#!/usr/bin/env python3
"""运行真实 Chromium 200 节点预算并产出任务 39 证据。"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
from typing import Final, Sequence

if __package__:
    from .capacity import git_revision, require_git_revision
else:
    from capacity import git_revision, require_git_revision


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT: Final = ROOT / "artifacts" / "capacity" / "web-report.json"


def allocate_available_loopback_port() -> int:
    """让操作系统选择当前可绑定的回环端口，避免固定端口与本机软件冲突。"""

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    if not isinstance(port, int) or not 1 <= port <= 65_535:
        raise RuntimeError("操作系统没有返回有效的浏览器容量端口。")
    return port


def run() -> dict[str, object]:
    executable = shutil.which("corepack.cmd" if os.name == "nt" else "corepack")
    if executable is None:
        raise RuntimeError("缺少 Corepack，无法运行浏览器容量场景。")
    revision = git_revision()
    environment = os.environ.copy()
    environment["AGENT_ROOM_CAPACITY_REPORT"] = "1"
    environment["AGENT_ROOM_CAPACITY_REVISION"] = revision
    environment["AGENT_ROOM_E2E_PORT"] = str(allocate_available_loopback_port())
    REPORT.unlink(missing_ok=True)
    build = subprocess.run(
        [
            executable,
            "pnpm@10.28.0",
            "--filter",
            "@agent-room/web",
            "exec",
            "vite",
            "build",
            "--mode",
            "capacity",
        ],
        cwd=ROOT,
        env=environment,
        check=False,
    )
    if build.returncode != 0:
        raise RuntimeError("容量专用生产 Web 构建失败。")
    result = subprocess.run(
        [
            executable,
            "pnpm@10.28.0",
            "--filter",
            "@agent-room/web",
            "exec",
            "playwright",
            "test",
            "lobby-scene.e2e.ts",
            "--grep",
            "有界帧预算",
        ],
        cwd=ROOT,
        env=environment,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError("真实 Chromium 容量场景失败。")
    try:
        report = json.loads(REPORT.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError("浏览器容量场景没有生成有效报告。") from error
    if not isinstance(report, dict) or report.get("revision") != revision:
        raise RuntimeError("浏览器容量报告不是当前 Git 修订。")
    if report.get("passed") is not True:
        raise RuntimeError("浏览器容量报告未通过。")
    require_git_revision(revision)
    return report


def parse_args(arguments: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    parse_args(arguments)
    try:
        report = run()
        print(f"浏览器容量报告：{REPORT}")
        print(json.dumps(report["metrics"], ensure_ascii=False, indent=2))
        return 0
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
