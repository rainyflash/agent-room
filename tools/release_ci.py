#!/usr/bin/env python3
"""为 GitHub Actions 生成受约束且可复用的发布元数据。"""

from __future__ import annotations

import argparse
from datetime import UTC, datetime
import json
from pathlib import Path
import re
import sys
import time
import tomllib
from typing import Final, Mapping, Sequence

if __package__:
    from .release_promotion import initialize
else:
    from release_promotion import initialize


ROOT: Final = Path(__file__).resolve().parent.parent
TAG_PATTERN: Final = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:[-+][0-9A-Za-z.-]+)?$")
REVISION_PATTERN: Final = re.compile(r"^[0-9a-f]{40}$")


class ReleaseCiFailure(RuntimeError):
    """表示工作流输入无法形成可信发布元数据。"""


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--channel", choices=("stable", "testing"), required=True)
    parser.add_argument("--sequence", type=int, required=True)
    parser.add_argument("--rollback-from")
    parser.add_argument("--revision", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--workspace-manifest", type=Path, default=ROOT / "Cargo.toml")
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--promotion", type=Path, required=True)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--now-unix-seconds", type=int)
    return parser.parse_args(argv)


def workspace_version(path: Path) -> str:
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
        raise ReleaseCiFailure(f"无法读取 workspace manifest：{path}") from error
    workspace = document.get("workspace")
    package = workspace.get("package") if isinstance(workspace, dict) else None
    version = package.get("version") if isinstance(package, dict) else None
    if not isinstance(version, str) or not version:
        raise ReleaseCiFailure("workspace.package.version 缺失。")
    return version


def validate_repository(value: str) -> None:
    parts = value.split("/")
    if len(parts) != 2 or any(not re.fullmatch(r"[A-Za-z0-9_.-]+", part) for part in parts):
        raise ReleaseCiFailure("repository 必须是 owner/name。")


def build_metadata(args: argparse.Namespace) -> dict[str, object]:
    match = TAG_PATTERN.fullmatch(args.tag)
    if match is None:
        raise ReleaseCiFailure("发布标签必须是 v 开头的 SemVer。")
    version = args.tag.removeprefix("v")
    if version != workspace_version(args.workspace_manifest):
        raise ReleaseCiFailure("发布标签版本与 Cargo workspace 版本不一致。")
    if args.sequence <= 0:
        raise ReleaseCiFailure("发布序号必须大于零。")
    if not REVISION_PATTERN.fullmatch(args.revision):
        raise ReleaseCiFailure("revision 必须是完整 Git SHA。")
    validate_repository(args.repository)
    now = args.now_unix_seconds if args.now_unix_seconds is not None else int(time.time())
    if now < 0:
        raise ReleaseCiFailure("发布时间不能为负数。")
    base_url = f"https://github.com/{args.repository}/releases/download/{args.tag}"
    tauri_name = f"agent-room-update-v{version}.json"
    return {
        "schemaVersion": 1,
        "version": version,
        "tag": args.tag,
        "channel": args.channel,
        "sequence": args.sequence,
        "rollbackFrom": args.rollback_from or None,
        "revision": args.revision,
        "publishedAtUnixSeconds": now,
        "expiresAtUnixSeconds": now + 7 * 24 * 60 * 60,
        "releaseBaseUrl": base_url,
        "imageBase": f"ghcr.io/{args.repository.lower()}",
        "tauriManifestName": tauri_name,
        "tauriManifestUrl": f"{base_url}/{tauri_name}",
    }


def write_new_json(path: Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(value, target, ensure_ascii=False, indent=2)
            target.write("\n")
    except FileExistsError as error:
        raise ReleaseCiFailure(f"拒绝覆盖已有发布元数据：{path}") from error


def github_output_values(metadata: Mapping[str, object]) -> tuple[tuple[str, str], ...]:
    mappings = (
        ("version", "version"),
        ("tag", "tag"),
        ("channel", "channel"),
        ("sequence", "sequence"),
        ("revision", "revision"),
        ("published_at", "publishedAtUnixSeconds"),
        ("expires_at", "expiresAtUnixSeconds"),
        ("release_base_url", "releaseBaseUrl"),
        ("image_base", "imageBase"),
        ("tauri_manifest_name", "tauriManifestName"),
        ("tauri_manifest_url", "tauriManifestUrl"),
    )
    values = [(output, str(metadata[source])) for output, source in mappings]
    rollback = metadata.get("rollbackFrom")
    values.append(("rollback_from", rollback if isinstance(rollback, str) else ""))
    return tuple(values)


def append_github_output(path: Path, metadata: Mapping[str, object]) -> None:
    with path.open("a", encoding="utf-8", newline="\n") as target:
        for key, value in github_output_values(metadata):
            if "\n" in value or "\r" in value:
                raise ReleaseCiFailure(f"GitHub 输出 {key} 含换行。")
            target.write(f"{key}={value}\n")


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        metadata = build_metadata(args)
        write_new_json(args.metadata, metadata)
        initialize(
            str(metadata["version"]),
            str(metadata["revision"]),
            args.promotion,
        )
        if args.github_output is not None:
            append_github_output(args.github_output, metadata)
        published = datetime.fromtimestamp(int(metadata["publishedAtUnixSeconds"]), UTC)
        print(f"候选发布元数据已生成：{metadata['tag']} / {published.isoformat()}")
        return 0
    except (ReleaseCiFailure, OSError) as error:
        print(f"发布工作流输入失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
