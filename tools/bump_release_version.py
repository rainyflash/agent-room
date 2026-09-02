#!/usr/bin/env python3
"""一致更新 Agent Room 发布版本，拒绝遗漏受管版本入口。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from typing import Final, Sequence


ROOT: Final = Path(__file__).resolve().parent.parent
VERSION_PATTERN: Final = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
JSON_VERSION_FILES: Final = (
    Path("apps/desktop/package.json"),
    Path("apps/web/package.json"),
    Path("packages/protocol/package.json"),
    Path("packages/protocol-types/package.json"),
    Path("packages/ui-system/package.json"),
    Path("plugins/agent-room/.codex-plugin/plugin.json"),
    Path("apps/desktop/src-tauri/tauri.conf.json"),
)
TEXT_VERSION_FILES: Final = (
    Path("Cargo.toml"),
    Path(".github/workflows/ci.yml"),
    Path("README.md"),
    Path("README.zh-CN.md"),
    Path("docs/compatibility.md"),
)


class VersionBumpFailure(RuntimeError):
    """表示仓库版本入口不一致，无法安全升版。"""


def workspace_version(root: Path) -> str:
    manifest = root / "Cargo.toml"
    try:
        document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        version = document["workspace"]["package"]["version"]
    except (OSError, UnicodeError, tomllib.TOMLDecodeError, KeyError, TypeError) as error:
        raise VersionBumpFailure("无法读取 Cargo workspace 版本。") from error
    if not isinstance(version, str) or VERSION_PATTERN.fullmatch(version) is None:
        raise VersionBumpFailure("Cargo workspace 版本不是有效 SemVer。")
    return version


def replace_text_version(path: Path, old: str, new: str) -> None:
    source = path.read_text(encoding="utf-8")
    count = source.count(old)
    if count == 0:
        raise VersionBumpFailure(f"受管文件缺少当前版本 {old}：{path}")
    path.write_text(source.replace(old, new), encoding="utf-8", newline="\n")


def replace_json_version(path: Path, old: str, new: str) -> None:
    try:
        source = path.read_text(encoding="utf-8")
        document = json.loads(source)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise VersionBumpFailure(f"无法读取 JSON：{path}") from error
    if not isinstance(document, dict) or document.get("version") != old:
        raise VersionBumpFailure(f"JSON version 与 workspace 不一致：{path}")
    pattern = re.compile(rf'("version"\s*:\s*"){re.escape(old)}(")')
    updated, count = pattern.subn(rf"\g<1>{new}\g<2>", source)
    if count != 1:
        raise VersionBumpFailure(f"JSON version 字段必须唯一：{path}")
    path.write_text(updated, encoding="utf-8", newline="\n")


def validate_managed_files(root: Path, old: str) -> None:
    for relative in JSON_VERSION_FILES:
        path = root / relative
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise VersionBumpFailure(f"无法读取 JSON：{path}") from error
        if not isinstance(document, dict) or document.get("version") != old:
            raise VersionBumpFailure(f"JSON version 与 workspace 不一致：{path}")
    for relative in TEXT_VERSION_FILES:
        path = root / relative
        try:
            source = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            raise VersionBumpFailure(f"无法读取受管文件：{path}") from error
        if old not in source:
            raise VersionBumpFailure(f"受管文件缺少当前版本 {old}：{path}")


def refresh_cargo_lock(root: Path) -> None:
    result = subprocess.run(
        ["cargo", "check", "--workspace", "--all-targets", "--all-features"],
        cwd=root,
        check=False,
    )
    if result.returncode != 0:
        raise VersionBumpFailure("Cargo.lock 刷新或 workspace 校验失败。")


def bump(root: Path, new: str, *, refresh_lock: bool = True) -> str:
    if VERSION_PATTERN.fullmatch(new) is None:
        raise VersionBumpFailure("目标版本不是有效 SemVer。")
    old = workspace_version(root)
    if old == new:
        raise VersionBumpFailure("目标版本与当前版本相同。")
    validate_managed_files(root, old)
    for relative in JSON_VERSION_FILES:
        replace_json_version(root / relative, old, new)
    for relative in TEXT_VERSION_FILES:
        replace_text_version(root / relative, old, new)
    if refresh_lock:
        refresh_cargo_lock(root)
    return old


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="不带 v 前缀的目标 SemVer")
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--skip-cargo", action="store_true", help=argparse.SUPPRESS)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        old = bump(args.root.resolve(), args.version, refresh_lock=not args.skip_cargo)
        print(f"发布版本已统一更新：{old} -> {args.version}")
        return 0
    except (VersionBumpFailure, OSError) as error:
        print(f"升版失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
