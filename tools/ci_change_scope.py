#!/usr/bin/env python3
"""判断这次 CI 是否只改了文档。

只有拉取请求、且改动全是文档时才走快速路径：quality 只做格式、凭据、许可证与 Python 工具检查，
Windows 原生与真实浏览器作业跳过。推送到 main 和发布派发永远完整运行。判断不出来时按「有代码改动」
处理，宁可多跑也不漏跑。
"""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
from typing import Final, Iterable

DOC_DIRECTORIES: Final = ("docs/", "specs/")
# 这些 Markdown 由工具生成或校验，改动它们要走完整检查。
GENERATED_MARKDOWN: Final = frozenset({"THIRD_PARTY_NOTICES.md"})


def is_documentation(path: str) -> bool:
    if path in GENERATED_MARKDOWN:
        return False
    if path.startswith(DOC_DIRECTORIES):
        return True
    return "/" not in path and path.endswith(".md")


def docs_only(paths: Iterable[str]) -> bool:
    changed = [path for path in paths if path]
    return bool(changed) and all(is_documentation(path) for path in changed)


def changed_paths() -> list[str]:
    # pull_request 检出的是合并提交：第一个父提交是目标分支，比较两者正好是这个 PR 的改动。
    output = subprocess.run(
        ["git", "diff", "--name-only", "HEAD^1", "HEAD"],
        capture_output=True, text=True, encoding="utf-8", check=True,
    ).stdout
    return output.splitlines()


def main() -> int:
    scope = False
    if os.environ.get("GITHUB_EVENT_NAME") == "pull_request":
        try:
            scope = docs_only(changed_paths())
        except (OSError, subprocess.CalledProcessError) as error:
            print(f"无法判断改动范围，按有代码改动处理：{error}", file=sys.stderr)
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"docs_only={'true' if scope else 'false'}\n")
    print("只改了文档，走快速检查。" if scope else "包含代码改动，完整检查。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
