#!/usr/bin/env python3
"""运行真实 Chromium 200 节点预算并产出任务 39 证据。"""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Final

if __package__:
    from .capacity import git_revision
else:
    from capacity import git_revision


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT: Final = ROOT / "artifacts" / "capacity" / "web-report.json"


def run() -> dict[str, object]:
    executable = shutil.which("corepack.cmd" if os.name == "nt" else "corepack")
    if executable is None:
        raise RuntimeError("缺少 Corepack，无法运行浏览器容量场景。")
    revision = git_revision()
    environment = os.environ.copy()
    environment["AGENT_ROOM_CAPACITY_REPORT"] = "1"
    environment["AGENT_ROOM_CAPACITY_REVISION"] = revision
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
    return report


def main() -> int:
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
