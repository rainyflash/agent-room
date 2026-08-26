#!/usr/bin/env python3
"""把 Go/No-Go 阻断同步为可审计的 GitHub Issue。"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Final, Iterable, Mapping, Sequence

if __package__:
    from .go_no_go import ROOT, GoNoGoFailure, load_decision, validate_decision
else:
    from go_no_go import ROOT, GoNoGoFailure, load_decision, validate_decision


MARKER_PATTERN: Final = re.compile(r"<!-- agent-room-go-no-go:(GNG-[0-9]{3}) -->")
LABELS: Final = (
    ("release-blocker", "B60205", "阻止 Agent Room 公开发行的发布门禁"),
    ("go-no-go", "5319E7", "由 Go/No-Go 唯一事实源自动同步"),
)


class ReadinessIssueError(RuntimeError):
    """表示 GitHub 跟踪状态无法从 Go/No-Go 事实源安全收敛。"""


@dataclass(frozen=True, slots=True)
class BlockerIssue:
    identifier: str
    title: str
    owner: str
    exit_condition: str
    evidence: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class ExistingIssue:
    number: int
    state: str
    identifier: str


def parse_blockers(value: Mapping[str, object]) -> tuple[BlockerIssue, ...]:
    validate_decision(value)
    raw_blockers = value.get("blockers")
    if not isinstance(raw_blockers, list):
        raise ReadinessIssueError("Go/No-Go blockers 必须是数组。")
    blockers: list[BlockerIssue] = []
    for raw in raw_blockers:
        if not isinstance(raw, dict):
            raise ReadinessIssueError("Go/No-Go blocker 必须是对象。")
        blockers.append(
            BlockerIssue(
                identifier=_text(raw, "id"),
                title=_text(raw, "title"),
                owner=_text(raw, "owner"),
                exit_condition=_text(raw, "exitCondition"),
                evidence=_strings(raw, "evidence"),
            )
        )
    return tuple(sorted(blockers, key=lambda blocker: blocker.identifier))


def issue_title(blocker: BlockerIssue) -> str:
    return f"[{blocker.identifier}] {blocker.title}"


def render_body(blocker: BlockerIssue, repository: str, revision: str) -> str:
    evidence = "\n".join(
        f"- [`{path}`](https://github.com/{repository}/blob/{revision}/{path})"
        for path in blocker.evidence
    )
    return f"""<!-- agent-room-go-no-go:{blocker.identifier} -->
此 Issue 由 `release/go-no-go/public-beta.json` 自动同步。不要手工维护第二套结论，也不要在这里披露未修复漏洞、凭据或私人消息内容。

## 责任角色

{blocker.owner}

## 解除条件

{blocker.exit_condition}

## 当前证据

{evidence}

## 完成规则

只有证据已经进入受版本控制的不可变提交、相关自动门禁通过，并从 Go/No-Go 事实源移除该阻断后，本 Issue 才会由同步器关闭。口头确认、截图和缩短测试不能替代证据。
"""


def parse_existing_issues(value: object) -> tuple[ExistingIssue, ...]:
    if not isinstance(value, list):
        raise ReadinessIssueError("GitHub Issue 列表不是数组。")
    parsed: list[ExistingIssue] = []
    seen: set[str] = set()
    for raw in value:
        if not isinstance(raw, dict):
            raise ReadinessIssueError("GitHub Issue 记录不是对象。")
        body = raw.get("body")
        if not isinstance(body, str):
            continue
        marker = MARKER_PATTERN.search(body)
        if marker is None:
            continue
        identifier = marker.group(1)
        if identifier in seen:
            raise ReadinessIssueError(f"GitHub 中存在重复阻断 Issue：{identifier}。")
        number = raw.get("number")
        state = raw.get("state")
        if not isinstance(number, int) or not isinstance(state, str):
            raise ReadinessIssueError(f"GitHub 阻断 Issue {identifier} 元数据无效。")
        seen.add(identifier)
        parsed.append(ExistingIssue(number, state.upper(), identifier))
    return tuple(sorted(parsed, key=lambda issue: issue.identifier))


def synchronize(repository: str, blockers: Sequence[BlockerIssue], revision: str) -> None:
    for name, color, description in LABELS:
        _run_gh(
            (
                "label",
                "create",
                name,
                "--repo",
                repository,
                "--color",
                color,
                "--description",
                description,
                "--force",
            )
        )

    raw_existing = json.loads(
        _run_gh(
            (
                "issue",
                "list",
                "--repo",
                repository,
                "--state",
                "all",
                "--limit",
                "1000",
                "--json",
                "number,state,body",
            )
        )
    )
    existing = {issue.identifier: issue for issue in parse_existing_issues(raw_existing)}
    active = {blocker.identifier: blocker for blocker in blockers}

    for identifier, blocker in active.items():
        body = render_body(blocker, repository, revision)
        issue = existing.get(identifier)
        if issue is None:
            url = _run_gh(
                (
                    "issue",
                    "create",
                    "--repo",
                    repository,
                    "--title",
                    issue_title(blocker),
                    "--body-file",
                    "-",
                    "--label",
                    "release-blocker,go-no-go",
                ),
                input_text=body,
            ).strip()
            print(f"创建 {identifier}：{url}")
            continue

        _run_gh(
            (
                "issue",
                "edit",
                str(issue.number),
                "--repo",
                repository,
                "--title",
                issue_title(blocker),
                "--body-file",
                "-",
                "--add-label",
                "release-blocker,go-no-go",
            ),
            input_text=body,
        )
        if issue.state == "CLOSED":
            _run_gh(("issue", "reopen", str(issue.number), "--repo", repository))
        print(f"更新 {identifier}：#{issue.number}")

    for identifier, issue in existing.items():
        if identifier in active or issue.state == "CLOSED":
            continue
        _run_gh(
            (
                "issue",
                "close",
                str(issue.number),
                "--repo",
                repository,
                "--comment",
                "Go/No-Go 唯一事实源已移除此阻断；同步器关闭跟踪 Issue。",
            )
        )
        print(f"关闭 {identifier}：#{issue.number}")


def render_plan(blockers: Iterable[BlockerIssue], repository: str, revision: str) -> str:
    return json.dumps(
        {
            "repository": repository,
            "revision": revision,
            "issues": [
                {
                    "id": blocker.identifier,
                    "title": issue_title(blocker),
                    "owner": blocker.owner,
                    "evidence": list(blocker.evidence),
                }
                for blocker in blockers
            ],
        },
        ensure_ascii=False,
        indent=2,
    )


def git_revision() -> str:
    return _run_command(("git", "rev-parse", "HEAD")).strip()


def _run_gh(arguments: Sequence[str], *, input_text: str | None = None) -> str:
    return _run_command(("gh", *arguments), input_text=input_text)


def _run_command(arguments: Sequence[str], *, input_text: str | None = None) -> str:
    completed = subprocess.run(
        list(arguments),
        cwd=ROOT,
        input=input_text,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "无错误输出"
        raise ReadinessIssueError(f"命令失败（{' '.join(arguments)}）：{detail}")
    return completed.stdout


def _text(value: Mapping[str, object], key: str) -> str:
    item = value.get(key)
    if not isinstance(item, str) or not item.strip():
        raise ReadinessIssueError(f"字段 {key} 必须是非空字符串。")
    return item.strip()


def _strings(value: Mapping[str, object], key: str) -> tuple[str, ...]:
    item = value.get(key)
    if not isinstance(item, list) or not item or any(not isinstance(child, str) for child in item):
        raise ReadinessIssueError(f"字段 {key} 必须是非空字符串数组。")
    return tuple(item)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("plan", "sync"), nargs="?", default="plan")
    parser.add_argument("--repo", default="rainyflash/agent-room")
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    try:
        blockers = parse_blockers(load_decision())
        revision = git_revision()
        if arguments.action == "plan":
            print(render_plan(blockers, arguments.repo, revision))
        else:
            synchronize(arguments.repo, blockers, revision)
        return 0
    except (GoNoGoFailure, ReadinessIssueError, OSError, json.JSONDecodeError) as error:
        print(f"Go/No-Go Issue 同步失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
