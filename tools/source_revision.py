"""读取可用于验收证据的干净 Git 源码修订。"""

from __future__ import annotations

from pathlib import Path
import re
import subprocess
from typing import Final, Sequence


REVISION_PATTERN: Final = re.compile(r"[0-9a-f]{40}|[0-9a-f]{64}")


class SourceRevisionFailure(RuntimeError):
    """表示当前源码状态不能生成可信验收证据。"""


def _run_git(root: Path, arguments: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *arguments],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )


def clean_git_revision(root: Path) -> str:
    """返回完整提交哈希，并拒绝任何已跟踪或未跟踪的源码改动。"""

    status = _run_git(
        root,
        (
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--ignore-submodules=none",
        ),
    )
    if status.returncode != 0:
        detail = status.stderr.strip() or status.stdout.strip() or "无错误输出"
        raise SourceRevisionFailure(f"无法检查 Git 工作树：{detail}")

    changes = tuple(line for line in status.stdout.splitlines() if line.strip())
    if changes:
        preview = "；".join(changes[:8])
        suffix = "；其余变更已省略" if len(changes) > 8 else ""
        raise SourceRevisionFailure(
            f"验收证据只能从干净 Git 工作树生成：{preview}{suffix}"
        )

    revision_result = _run_git(root, ("rev-parse", "--verify", "HEAD^{commit}"))
    revision = revision_result.stdout.strip()
    if revision_result.returncode != 0 or REVISION_PATTERN.fullmatch(revision) is None:
        detail = (
            revision_result.stderr.strip()
            or revision_result.stdout.strip()
            or "无错误输出"
        )
        raise SourceRevisionFailure(f"无法读取完整 Git 提交：{detail}")
    return revision


def require_clean_git_revision(root: Path, expected: str) -> None:
    """确保长时间验收结束时仍处于开始时的干净源码修订。"""

    actual = clean_git_revision(root)
    if actual != expected:
        raise SourceRevisionFailure(
            f"Git 修订在验收期间发生变化：开始 {expected}，结束 {actual}。"
        )
