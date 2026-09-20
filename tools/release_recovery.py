#!/usr/bin/env python3
"""Reuse completed native/image artifacts without changing a candidate's identity."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from typing import Mapping, Sequence

if __package__:
    from .release_ci import (
        ROOT, ReleaseCiFailure, append_github_output, build_metadata,
        validate_repository, write_new_json,
    )
    from .release_promotion import initialize
else:
    from release_ci import (
        ROOT, ReleaseCiFailure, append_github_output, build_metadata,
        validate_repository, write_new_json,
    )
    from release_promotion import initialize


def validate_source(run: Mapping[str, object], artifacts: Sequence[object],
                    repository: str, profile: str) -> str:
    """Only recover this repository's completed, protected-main candidate builds."""
    source_repository = run.get("head_repository")
    run_repository = run.get("repository")
    revision = run.get("head_sha")
    if (
        not isinstance(source_repository, dict)
        or source_repository.get("full_name") != repository
        or not isinstance(run_repository, dict)
        or run_repository.get("full_name") != repository
        or run.get("path") != ".github/workflows/release-candidate.yml"
        or run.get("head_branch") != "main"
        or run.get("event") != "workflow_dispatch"
        or run.get("status") != "completed"
        or not isinstance(revision, str)
        or re.fullmatch(r"[0-9a-f]{40}", revision) is None
    ):
        raise ReleaseCiFailure("恢复来源必须是本仓库已结束的 main 候选工作流。")
    if profile not in ("client", "full"):
        raise ReleaseCiFailure("恢复 profile 无效。")
    names: list[str] = []
    for artifact in artifacts:
        if not isinstance(artifact, dict) or not isinstance(artifact.get("name"), str):
            raise ReleaseCiFailure("来源产物列表无效。")
        name = artifact["name"]
        if name.startswith("release-candidate-"):
            raise ReleaseCiFailure("完整候选已保存；请直接验证并恢复其上传，不要重新签署。")
        if name.startswith("release-"):
            if artifact.get("expired") is not False:
                raise ReleaseCiFailure("来源候选产物已过期。")
            names.append(name)
    required = {"release-metadata", "release-native-windows-x86_64", "release-native-darwin-aarch64"}
    if profile == "full":
        required.update(f"release-image-{name}" for name in ("control-plane", "identity", "web"))
    if len(names) != len(set(names)) or set(names) != required:
        raise ReleaseCiFailure("候选产物缺失、重复或与发行 profile 不一致。")
    return revision


def validate_metadata(metadata: Mapping[str, object], args: argparse.Namespace,
                      revision: str, now: int) -> dict[str, object]:
    published = metadata.get("publishedAtUnixSeconds")
    if isinstance(published, bool) or not isinstance(published, int):
        raise ReleaseCiFailure("来源候选发布时间无效。")
    expected = build_metadata(argparse.Namespace(
        tag=args.tag, channel=args.channel, sequence=args.sequence,
        rollback_from=args.rollback_from, revision=revision, repository=args.repository,
        workspace_manifest=args.workspace_manifest, now_unix_seconds=published,
    ))
    if dict(metadata) != expected:
        raise ReleaseCiFailure("来源候选身份、地址、序号或有效期与请求不一致。")
    expires = expected["expiresAtUnixSeconds"]
    if not isinstance(expires, int) or not published <= now < expires:
        raise ReleaseCiFailure("来源候选尚未生效或已过期；恢复不能延长有效期。")
    return expected


def run_checked(command: Sequence[str]) -> str:
    result = subprocess.run(command, check=False, capture_output=True, text=True,
                            encoding="utf-8", timeout=180, cwd=ROOT)
    if result.returncode != 0:
        raise ReleaseCiFailure(
            f"恢复步骤失败：{command[0]}（退出码 {result.returncode}）："
            f"{result.stderr.strip()[:1500]}"
        )
    return result.stdout


def read_object(value: str) -> dict[str, object]:
    parsed = json.loads(value)
    if not isinstance(parsed, dict):
        raise ReleaseCiFailure("恢复数据必须是 JSON 对象。")
    return parsed


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--channel", choices=("testing",), required=True)
    parser.add_argument("--sequence", type=int, required=True)
    parser.add_argument("--profile", choices=("client", "full"), required=True)
    parser.add_argument("--rollback-from", default="")
    parser.add_argument("--workspace-manifest", type=Path, default=ROOT / "Cargo.toml")
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--promotion", type=Path, required=True)
    parser.add_argument("--github-output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        validate_repository(args.repository)
        if re.fullmatch(r"[1-9][0-9]{0,19}", args.run_id) is None:
            raise ReleaseCiFailure("run-id 必须是正整数。")
        endpoint = f"repos/{args.repository}/actions/runs/{args.run_id}"
        run = read_object(run_checked(("gh", "api", endpoint)))
        inventory = read_object(run_checked(("gh", "api", f"{endpoint}/artifacts?per_page=100")))
        artifacts = inventory.get("artifacts")
        if not isinstance(artifacts, list) or inventory.get("total_count") != len(artifacts):
            raise ReleaseCiFailure("来源产物列表不完整。")
        revision = validate_source(run, artifacts, args.repository, args.profile)
        run_checked(("git", "merge-base", "--is-ancestor", revision, "HEAD"))
        with tempfile.TemporaryDirectory(prefix="agent-room-release-recovery-") as directory:
            run_checked(("gh", "run", "download", args.run_id, "--repo", args.repository,
                         "--name", "release-metadata", "--dir", directory))
            metadata = validate_metadata(
                read_object(Path(directory, "release-metadata.json").read_text(encoding="utf-8")),
                args, revision, int(time.time()),
            )
        write_new_json(args.metadata, metadata)
        initialize(str(metadata["version"]), revision, args.promotion)
        append_github_output(args.github_output, metadata)
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"artifact_run_id={args.run_id}\n")
        print(f"复用已完成的候选产物：{args.run_id} / {revision}；不改版本、序号或有效期。")
        return 0
    except (ReleaseCiFailure, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"候选恢复失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
