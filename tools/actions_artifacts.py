#!/usr/bin/env python3
"""审计并清理 Agent Room 仓库中过时的 GitHub Actions 临时制品。"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Final, Mapping, Sequence
from urllib.parse import quote


ROOT: Final = Path(__file__).resolve().parent.parent
RELEASE_ARTIFACT_PREFIX: Final = "release-"


class ArtifactRetentionFailure(RuntimeError):
    """表示远端制品事实不完整，或清理操作失败。"""


@dataclass(frozen=True, slots=True)
class Artifact:
    identifier: int
    name: str
    size_bytes: int
    created_at: str
    run_id: int
    head_sha: str


@dataclass(frozen=True, slots=True)
class WorkflowRun:
    identifier: int
    status: str
    conclusion: str | None


@dataclass(frozen=True, slots=True)
class RetentionDecision:
    artifact: Artifact
    keep: bool
    reason: str


def retention_decisions(
    artifacts: Sequence[Artifact],
    runs: Mapping[int, WorkflowRun],
    current_revision: str,
) -> tuple[RetentionDecision, ...]:
    """保留发布物、当前修订、活跃运行及每个名称的最新与最近成功制品。"""

    latest: dict[str, Artifact] = {}
    latest_successful: dict[str, Artifact] = {}
    for artifact in artifacts:
        run = runs.get(artifact.run_id)
        if run is None:
            raise ArtifactRetentionFailure(
                f"Artifact {artifact.identifier} 缺少 Workflow Run {artifact.run_id}。"
            )
        current_latest = latest.get(artifact.name)
        if current_latest is None or (
            artifact.created_at,
            artifact.identifier,
        ) > (current_latest.created_at, current_latest.identifier):
            latest[artifact.name] = artifact
        if run.status == "completed" and run.conclusion == "success":
            current = latest_successful.get(artifact.name)
            if current is None or (
                artifact.created_at,
                artifact.identifier,
            ) > (current.created_at, current.identifier):
                latest_successful[artifact.name] = artifact

    decisions: list[RetentionDecision] = []
    for artifact in artifacts:
        run = runs[artifact.run_id]
        if artifact.name.startswith(RELEASE_ARTIFACT_PREFIX):
            reason = "release_artifact"
        elif artifact.head_sha == current_revision:
            reason = "current_revision"
        elif run.status != "completed":
            reason = "active_workflow"
        elif latest.get(artifact.name) == artifact:
            reason = "latest_by_name"
        elif latest_successful.get(artifact.name) == artifact:
            reason = "latest_successful_by_name"
        else:
            decisions.append(
                RetentionDecision(artifact=artifact, keep=False, reason="stale")
            )
            continue
        decisions.append(RetentionDecision(artifact=artifact, keep=True, reason=reason))
    return tuple(decisions)


def required_string(value: Mapping[str, object], key: str, label: str) -> str:
    item = value.get(key)
    if not isinstance(item, str) or not item:
        raise ArtifactRetentionFailure(f"{label} 缺少字符串字段 {key}。")
    return item


def required_integer(value: Mapping[str, object], key: str, label: str) -> int:
    item = value.get(key)
    if not isinstance(item, int) or isinstance(item, bool):
        raise ArtifactRetentionFailure(f"{label} 缺少整数字段 {key}。")
    return item


def parse_artifacts(pages: object) -> tuple[Artifact, ...]:
    if not isinstance(pages, list):
        raise ArtifactRetentionFailure("Artifact 分页响应必须是数组。")
    parsed: list[Artifact] = []
    for page in pages:
        if not isinstance(page, dict):
            raise ArtifactRetentionFailure("Artifact 分页响应包含非对象页面。")
        values = page.get("artifacts")
        if not isinstance(values, list):
            raise ArtifactRetentionFailure("Artifact 页面缺少 artifacts 数组。")
        for value in values:
            if not isinstance(value, dict):
                raise ArtifactRetentionFailure("Artifact 页面包含非对象条目。")
            label = f"Artifact {value.get('id', 'unknown')}"
            workflow_run = value.get("workflow_run")
            if not isinstance(workflow_run, dict):
                raise ArtifactRetentionFailure(f"{label} 缺少 workflow_run。")
            parsed.append(
                Artifact(
                    identifier=required_integer(value, "id", label),
                    name=required_string(value, "name", label),
                    size_bytes=required_integer(value, "size_in_bytes", label),
                    created_at=required_string(value, "created_at", label),
                    run_id=required_integer(workflow_run, "id", label),
                    head_sha=required_string(workflow_run, "head_sha", label),
                )
            )
    return tuple(parsed)


def parse_workflow_run(value: object, expected_identifier: int) -> WorkflowRun:
    if not isinstance(value, dict):
        raise ArtifactRetentionFailure("Workflow Run 响应必须是对象。")
    label = f"Workflow Run {expected_identifier}"
    identifier = required_integer(value, "id", label)
    if identifier != expected_identifier:
        raise ArtifactRetentionFailure(
            f"Workflow Run ID 不一致：期望 {expected_identifier}，实际 {identifier}。"
        )
    conclusion = value.get("conclusion")
    if conclusion is not None and not isinstance(conclusion, str):
        raise ArtifactRetentionFailure(f"{label} 的 conclusion 必须是字符串或 null。")
    return WorkflowRun(
        identifier=identifier,
        status=required_string(value, "status", label),
        conclusion=conclusion,
    )


def run_gh(arguments: Sequence[str]) -> str:
    completed = subprocess.run(
        ["gh", *arguments],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "无错误输出"
        raise ArtifactRetentionFailure(
            f"GitHub CLI 命令失败（gh {' '.join(arguments)}）：{detail}"
        )
    return completed.stdout


def gh_json(arguments: Sequence[str]) -> object:
    output = run_gh(arguments)
    try:
        return json.loads(output)
    except json.JSONDecodeError as error:
        raise ArtifactRetentionFailure("GitHub CLI 没有返回有效 JSON。") from error


def resolve_repository(explicit: str | None) -> str:
    candidate = explicit or os.environ.get("GITHUB_REPOSITORY")
    if candidate:
        return candidate
    value = gh_json(("repo", "view", "--json", "nameWithOwner"))
    if not isinstance(value, dict):
        raise ArtifactRetentionFailure("无法推导 GitHub 仓库。")
    return required_string(value, "nameWithOwner", "GitHub Repository")


def current_default_revision(repository: str) -> str:
    metadata = gh_json(("api", f"repos/{repository}"))
    if not isinstance(metadata, dict):
        raise ArtifactRetentionFailure("GitHub Repository 响应必须是对象。")
    default_branch = required_string(metadata, "default_branch", "GitHub Repository")
    commit = gh_json(
        ("api", f"repos/{repository}/commits/{quote(default_branch, safe='')}")
    )
    if not isinstance(commit, dict):
        raise ArtifactRetentionFailure("GitHub Commit 响应必须是对象。")
    return required_string(commit, "sha", "GitHub Commit")


def load_artifacts(repository: str) -> tuple[Artifact, ...]:
    pages = gh_json(
        (
            "api",
            "--paginate",
            "--slurp",
            f"repos/{repository}/actions/artifacts?per_page=100",
        )
    )
    return parse_artifacts(pages)


def load_runs(
    repository: str, artifacts: Sequence[Artifact]
) -> dict[int, WorkflowRun]:
    runs: dict[int, WorkflowRun] = {}
    for run_id in sorted({artifact.run_id for artifact in artifacts}):
        value = gh_json(("api", f"repos/{repository}/actions/runs/{run_id}"))
        runs[run_id] = parse_workflow_run(value, run_id)
    return runs


def delete_artifact(repository: str, identifier: int) -> None:
    run_gh(
        (
            "api",
            "--method",
            "DELETE",
            f"repos/{repository}/actions/artifacts/{identifier}",
        )
    )


def parse_args(arguments: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action", choices=("audit", "prune"), nargs="?", default="audit"
    )
    parser.add_argument("--repository")
    parser.add_argument("--keep-revision")
    parser.add_argument("--apply", action="store_true")
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_args(arguments)
    try:
        repository = resolve_repository(options.repository)
        revision = options.keep_revision or current_default_revision(repository)
        artifacts = load_artifacts(repository)
        runs = load_runs(repository, artifacts)
        decisions = retention_decisions(artifacts, runs, revision)
        stale = tuple(decision.artifact for decision in decisions if not decision.keep)
        stale_bytes = sum(artifact.size_bytes for artifact in stale)
        for artifact in stale:
            print(
                f"删除候选 {artifact.identifier}: {artifact.name} "
                f"({artifact.size_bytes} bytes, run {artifact.run_id}, "
                f"{artifact.head_sha})"
            )
        print(
            f"保留 {len(decisions) - len(stale)} 个，清理候选 {len(stale)} 个，"
            f"可释放 {stale_bytes / 1_048_576:.2f} MiB。"
        )
        if options.action == "prune" and options.apply:
            for artifact in stale:
                delete_artifact(repository, artifact.identifier)
            print("过时 Actions 临时制品已删除。")
        elif options.apply:
            raise ArtifactRetentionFailure("--apply 只能与 prune 动作一起使用。")
        return 0
    except ArtifactRetentionFailure as error:
        print(f"Actions 制品保留失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
