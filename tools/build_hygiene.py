#!/usr/bin/env python3
"""验证构建没有改写源码，并清理由 Tauri 生成的平台 Schema。"""

from __future__ import annotations

from pathlib import Path
import subprocess
import sys
from typing import Final, Sequence


ROOT: Final = Path(__file__).resolve().parent.parent
TAURI_SCHEMA_PATH: Final = "apps/desktop/src-tauri/gen/schemas"


class BuildHygieneFailure(RuntimeError):
    """表示构建越过允许的生成物边界，或清理过程失败。"""


def run_git(*arguments: str, capture_output: bool = False) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *arguments],
        cwd=ROOT,
        check=False,
        capture_output=capture_output,
        text=True,
    )


def verify_and_clean_tauri_schemas() -> None:
    """只允许 Tauri Schema 变化，恢复后再次要求整个工作树干净。"""

    schema_root = (ROOT / TAURI_SCHEMA_PATH).resolve()
    try:
        schema_root.relative_to(ROOT.resolve())
    except ValueError as error:
        raise BuildHygieneFailure("Tauri Schema 目录逃逸工作区。") from error

    source_diff = run_git(
        "diff",
        "--exit-code",
        "--",
        ".",
        f":(exclude){TAURI_SCHEMA_PATH}",
    )
    if source_diff.returncode != 0:
        raise BuildHygieneFailure("构建改写了 Tauri Schema 目录之外的源码。")

    for action in (
        ("restore", "--source=HEAD", "--", TAURI_SCHEMA_PATH),
        ("clean", "-fd", "--", TAURI_SCHEMA_PATH),
    ):
        result = run_git(*action)
        if result.returncode != 0:
            raise BuildHygieneFailure(f"Git 构建清理失败：{action[0]}。")

    status = run_git("status", "--porcelain", capture_output=True)
    if status.returncode != 0:
        raise BuildHygieneFailure("无法读取构建后的 Git 工作树状态。")
    dirty = status.stdout.strip()
    if dirty:
        raise BuildHygieneFailure(f"构建清理后工作树仍不干净：\n{dirty}")


def parse_args(arguments: Sequence[str] | None = None) -> None:
    if list(arguments or ()):
        raise BuildHygieneFailure("构建卫生检查不接受命令行参数。")


def main(arguments: Sequence[str] | None = None) -> int:
    try:
        parse_args(arguments)
        verify_and_clean_tauri_schemas()
        print("构建卫生检查通过；Tauri 平台 Schema 已恢复。")
        return 0
    except BuildHygieneFailure as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
