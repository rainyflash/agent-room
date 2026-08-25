#!/usr/bin/env python3
"""迁移并验证 Agent Room 自定义事件命名空间。"""

from __future__ import annotations

import argparse
import os
from pathlib import Path


LEGACY_NAMESPACE = "org.agentroom"
ACTIVE_NAMESPACE = "io.github.rainyflash.agentroom"
TEXT_SUFFIXES = {
    ".json",
    ".md",
    ".mjs",
    ".ps1",
    ".py",
    ".rs",
    ".sql",
    ".ts",
    ".tsx",
    ".yaml",
    ".yml",
}
IGNORED_DIRECTORIES = {
    ".git",
    ".local",
    ".venv",
    "artifacts",
    "coverage",
    "node_modules",
    "release",
    "target",
}


def repository_root() -> Path:
    return Path(__file__).resolve().parent.parent


def candidate_files(root: Path) -> list[Path]:
    script_path = Path(__file__).resolve()
    candidates: list[Path] = []
    for directory, child_directories, file_names in os.walk(root, topdown=True):
        child_directories[:] = sorted(
            name for name in child_directories if name not in IGNORED_DIRECTORIES
        )
        current = Path(directory)
        for file_name in sorted(file_names):
            path = current / file_name
            if path.resolve() == script_path or path.suffix.lower() not in TEXT_SUFFIXES:
                continue
            candidates.append(path)
    return candidates


def legacy_occurrences(root: Path) -> list[Path]:
    return [
        path
        for path in candidate_files(root)
        if LEGACY_NAMESPACE in path.read_text(encoding="utf-8")
    ]


def migrate(root: Path) -> list[Path]:
    changed: list[Path] = []
    for path in legacy_occurrences(root):
        content = path.read_text(encoding="utf-8")
        path.write_text(
            content.replace(LEGACY_NAMESPACE, ACTIVE_NAMESPACE),
            encoding="utf-8",
            newline="",
        )
        changed.append(path)
    return changed


def check(root: Path) -> int:
    stale = legacy_occurrences(root)
    if not stale:
        print(f"事件命名空间有效：{ACTIVE_NAMESPACE}")
        return 0

    print(f"发现已退役命名空间 {LEGACY_NAMESPACE}：")
    for path in stale:
        print(f"- {path.relative_to(root).as_posix()}")
    return 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="检查是否仍有旧命名空间")
    mode.add_argument("--write", action="store_true", help="迁移所有受控文本文件")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repository_root()
    if args.write:
        changed = migrate(root)
        print(f"已迁移 {len(changed)} 个文件到 {ACTIVE_NAMESPACE}")
    return check(root)


if __name__ == "__main__":
    raise SystemExit(main())
