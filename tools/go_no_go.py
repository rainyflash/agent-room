#!/usr/bin/env python3
"""验证并渲染公开测试 Go/No-Go 决策。"""

from __future__ import annotations

import argparse
from collections.abc import Mapping, Sequence
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
DECISION_PATH: Final = ROOT / "release" / "go-no-go" / "public-beta.json"
RECORD_PATH: Final = (
    ROOT / "specs" / "agent-room-foundation" / "task-45-go-no-go.md"
)
REQUIRED_REQUIREMENTS: Final = frozenset(range(1, 16))
REQUIRED_GATES: Final = frozenset(
    {
        "functional",
        "security",
        "capacity",
        "recovery",
        "federation",
        "deployment",
        "observability",
        "release",
        "open_source",
        "supply_chain",
    }
)
PUBLICATION_KEYS: Final = frozenset(
    {"releaseNotes", "knownLimitations", "dataPolicy", "securityPolicy"}
)
REVISION_PATTERN: Final = re.compile(r"^[0-9a-f]{40}$")


class GoNoGoFailure(RuntimeError):
    """表示决策记录缺失、矛盾或引用了无效证据。"""


def load_decision(path: Path = DECISION_PATH) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GoNoGoFailure(f"无法读取 Go/No-Go 决策：{path}") from error
    if not isinstance(value, dict):
        raise GoNoGoFailure("Go/No-Go 决策根节点必须是对象。")
    return value


def validate_decision(value: Mapping[str, object], root: Path = ROOT) -> None:
    if value.get("schemaVersion") != 1:
        raise GoNoGoFailure("Go/No-Go 决策 schemaVersion 必须为 1。")
    decision = required_string(value, "decision")
    if decision not in {"go", "no-go"}:
        raise GoNoGoFailure("decision 只能是 go 或 no-go。")
    required_string(value, "target")
    required_string(value, "recordedAt")
    revision = required_string(value, "baselineRevision")
    if REVISION_PATTERN.fullmatch(revision) is None:
        raise GoNoGoFailure("baselineRevision 必须是完整的小写 Git SHA。")
    enabled = value.get("publicBetaEnabled")
    if not isinstance(enabled, bool):
        raise GoNoGoFailure("publicBetaEnabled 必须是布尔值。")

    blockers = object_list(value, "blockers")
    blocker_ids = validate_blockers(blockers, root)
    requirements = object_list(value, "requirements")
    validate_requirements(requirements, blocker_ids, root)
    gates = object_list(value, "gates")
    validate_gates(gates, blocker_ids, root)
    validate_publication(value, root)

    blocked = any(item.get("status") == "blocked" for item in requirements + gates)
    if decision == "go":
        if blockers or blocked or enabled is not True:
            raise GoNoGoFailure("存在开放阻断、失败门禁或未启用公开测试时不得判定 Go。")
    elif not blockers or not blocked or enabled is not False:
        raise GoNoGoFailure("No-Go 必须保留至少一个明确阻断，并关闭公开测试开关。")


def validate_blockers(
    blockers: Sequence[Mapping[str, object]], root: Path
) -> frozenset[str]:
    identifiers: list[str] = []
    for blocker in blockers:
        identifier = required_string(blocker, "id")
        identifiers.append(identifier)
        if blocker.get("status") != "open" or blocker.get("severity") != "blocking":
            raise GoNoGoFailure(f"阻断 {identifier} 必须是 open/blocking。")
        required_string(blocker, "title")
        required_string(blocker, "owner")
        required_string(blocker, "exitCondition")
        validate_paths(string_list(blocker, "evidence"), root)
    if len(identifiers) != len(set(identifiers)):
        raise GoNoGoFailure("阻断 ID 不得重复。")
    return frozenset(identifiers)


def validate_requirements(
    requirements: Sequence[Mapping[str, object]],
    blocker_ids: frozenset[str],
    root: Path,
) -> None:
    identifiers: list[int] = []
    for requirement in requirements:
        identifier = requirement.get("id")
        if not isinstance(identifier, int) or isinstance(identifier, bool):
            raise GoNoGoFailure("需求 ID 必须是整数。")
        identifiers.append(identifier)
        required_string(requirement, "name")
        validate_status_and_blockers(requirement, blocker_ids, f"需求 {identifier}")
        validate_paths(string_list(requirement, "evidence"), root)
    if len(identifiers) != len(set(identifiers)):
        raise GoNoGoFailure("需求 ID 不得重复。")
    if frozenset(identifiers) != REQUIRED_REQUIREMENTS:
        missing = sorted(REQUIRED_REQUIREMENTS - frozenset(identifiers))
        extra = sorted(frozenset(identifiers) - REQUIRED_REQUIREMENTS)
        raise GoNoGoFailure(f"需求矩阵必须精确覆盖 1–15；缺少 {missing}，多出 {extra}。")


def validate_gates(
    gates: Sequence[Mapping[str, object]], blocker_ids: frozenset[str], root: Path
) -> None:
    identifiers: list[str] = []
    for gate in gates:
        identifier = required_string(gate, "id")
        identifiers.append(identifier)
        required_string(gate, "name")
        validate_status_and_blockers(gate, blocker_ids, f"门禁 {identifier}")
        validate_paths(string_list(gate, "evidence"), root)
    if len(identifiers) != len(set(identifiers)):
        raise GoNoGoFailure("门禁 ID 不得重复。")
    if frozenset(identifiers) != REQUIRED_GATES:
        missing = sorted(REQUIRED_GATES - frozenset(identifiers))
        extra = sorted(frozenset(identifiers) - REQUIRED_GATES)
        raise GoNoGoFailure(f"门禁集合不完整；缺少 {missing}，多出 {extra}。")


def validate_status_and_blockers(
    value: Mapping[str, object], blocker_ids: frozenset[str], label: str
) -> None:
    status = required_string(value, "status")
    references = frozenset(string_list(value, "blockerIds"))
    unknown = references - blocker_ids
    if unknown:
        raise GoNoGoFailure(f"{label} 引用了未知阻断：{sorted(unknown)}。")
    if status == "pass" and references:
        raise GoNoGoFailure(f"{label} 已通过，不得保留阻断引用。")
    if status == "blocked" and not references:
        raise GoNoGoFailure(f"{label} 被阻断时必须引用具体阻断。")
    if status not in {"pass", "blocked"}:
        raise GoNoGoFailure(f"{label} 状态只能是 pass 或 blocked。")


def validate_publication(value: Mapping[str, object], root: Path) -> None:
    publication = value.get("publication")
    if not isinstance(publication, dict):
        raise GoNoGoFailure("publication 必须是对象。")
    keys = frozenset(publication)
    if keys != PUBLICATION_KEYS:
        raise GoNoGoFailure(
            f"publication 必须精确包含 {sorted(PUBLICATION_KEYS)}。"
        )
    paths = []
    for key in sorted(PUBLICATION_KEYS):
        paths.append(required_string(publication, key))
    validate_paths(paths, root)


def validate_paths(paths: Sequence[str], root: Path) -> None:
    if not paths:
        raise GoNoGoFailure("每个需求、门禁或阻断必须至少引用一份证据。")
    resolved_root = root.resolve()
    for relative in paths:
        pure = PurePosixPath(relative)
        if pure.is_absolute() or ".." in pure.parts or "\\" in relative:
            raise GoNoGoFailure(f"证据路径必须是仓库内 POSIX 相对路径：{relative}")
        path = (resolved_root / Path(*pure.parts)).resolve()
        try:
            path.relative_to(resolved_root)
        except ValueError as error:
            raise GoNoGoFailure(f"证据路径越出仓库：{relative}") from error
        if not path.is_file():
            raise GoNoGoFailure(f"证据文件不存在：{relative}")


def required_string(value: Mapping[str, object], key: str) -> str:
    item = value.get(key)
    if not isinstance(item, str) or not item.strip():
        raise GoNoGoFailure(f"字段 {key} 必须是非空字符串。")
    return item


def string_list(value: Mapping[str, object], key: str) -> tuple[str, ...]:
    item = value.get(key)
    if not isinstance(item, list) or any(not isinstance(child, str) for child in item):
        raise GoNoGoFailure(f"字段 {key} 必须是字符串数组。")
    return tuple(item)


def object_list(
    value: Mapping[str, object], key: str
) -> tuple[Mapping[str, object], ...]:
    item = value.get(key)
    if not isinstance(item, list) or any(not isinstance(child, dict) for child in item):
        raise GoNoGoFailure(f"字段 {key} 必须是对象数组。")
    return tuple(item)


def render_record(
    value: Mapping[str, object], root: Path = ROOT, output: Path = RECORD_PATH
) -> str:
    validate_decision(value, root)
    requirements = object_list(value, "requirements")
    gates = object_list(value, "gates")
    blockers = object_list(value, "blockers")
    publication = value["publication"]
    assert isinstance(publication, dict)
    decision = required_string(value, "decision").upper()
    enabled = "是" if value["publicBetaEnabled"] is True else "否"
    lines = [
        "# 任务 45：公开测试 Go/No-Go 决策",
        "",
        "> 本文件由 `python tools/go_no_go.py generate` 从 "
        "[`release/go-no-go/public-beta.json`](../../release/go-no-go/public-beta.json) "
        "确定性生成，不得手工维护第二套结论。",
        "",
        "## 1. 决策",
        "",
        f"- **结论：{decision}**",
        f"- 目标：`{required_string(value, 'target')}`",
        f"- 记录日期：{required_string(value, 'recordedAt')}",
        f"- 验收基线：`{required_string(value, 'baselineRevision')}`",
        f"- 公开测试已启用：{enabled}",
        "",
        "本决定不是‘差不多可以上线’。开放阻断全部关闭且所有门禁变为通过之前，"
        "不得启用公共联邦、发布公开测试安装包或把当前代码描述为生产支持版本。",
        "",
        "## 2. 需求 1–15 验收矩阵",
        "",
        "| 需求 | 状态 | 证据 | 阻断 |",
        "| --- | --- | --- | --- |",
    ]
    for requirement in requirements:
        identifier = requirement["id"]
        name = required_string(requirement, "name")
        status = status_label(required_string(requirement, "status"))
        evidence = links(string_list(requirement, "evidence"), root, output)
        blocker_refs = ", ".join(string_list(requirement, "blockerIds")) or "—"
        lines.append(f"| {identifier} {name} | {status} | {evidence} | {blocker_refs} |")

    lines.extend(
        [
            "",
            "## 3. 发布门禁",
            "",
            "| 门禁 | 状态 | 证据 | 阻断 |",
            "| --- | --- | --- | --- |",
        ]
    )
    for gate in gates:
        name = required_string(gate, "name")
        status = status_label(required_string(gate, "status"))
        evidence = links(string_list(gate, "evidence"), root, output)
        blocker_refs = ", ".join(string_list(gate, "blockerIds")) or "—"
        lines.append(f"| {name} | {status} | {evidence} | {blocker_refs} |")

    lines.extend(["", "## 4. 开放阻断", ""])
    for blocker in blockers:
        identifier = required_string(blocker, "id")
        title = required_string(blocker, "title")
        owner = required_string(blocker, "owner")
        condition = required_string(blocker, "exitCondition")
        evidence = links(string_list(blocker, "evidence"), root, output)
        lines.extend(
            [
                f"### {identifier} · {title}",
                "",
                f"- 责任角色：{owner}",
                f"- 解除条件：{condition}",
                f"- 当前证据：{evidence}",
                "",
            ]
        )

    lines.extend(["## 5. 对外发布材料", ""])
    for key, label in (
        ("releaseNotes", "版本说明"),
        ("knownLimitations", "已知限制"),
        ("dataPolicy", "数据与保留策略"),
        ("securityPolicy", "安全联系方式"),
    ):
        path = required_string(publication, key)
        lines.append(f"- {label}：{links((path,), root, output)}")
    lines.extend(
        [
            "",
            "## 6. 重新评审规则",
            "",
            "只有在每个开放阻断均有不可变证据、需求和门禁全部为 `pass`、"
            "`publicBetaEnabled` 明确改为 `true` 后，才允许把 JSON 决策改成 `go`。"
            "执行 `python tools/go_no_go.py assert-go` 必须返回成功；人工口头批准不能绕过该门。",
            "",
        ]
    )
    return "\n".join(lines)


def links(paths: Sequence[str], root: Path, output: Path) -> str:
    rendered = []
    for relative in paths:
        target = root.joinpath(*PurePosixPath(relative).parts)
        link = Path(os.path.relpath(target, output.parent)).as_posix()
        rendered.append(f"[`{relative}`]({link})")
    return "<br>".join(rendered)


def status_label(status: str) -> str:
    return "通过" if status == "pass" else "**阻断**"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action", choices=("validate", "generate", "assert-go"), nargs="?", default="validate"
    )
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    try:
        value = load_decision()
        validate_decision(value)
        if arguments.action == "generate":
            RECORD_PATH.write_text(render_record(value), encoding="utf-8")
            print(f"Go/No-Go 决策记录已生成：{RECORD_PATH}")
            return 0
        if arguments.action == "assert-go" and value.get("decision") != "go":
            print("公开测试当前为 No-Go。", file=sys.stderr)
            return 2
        print(f"Go/No-Go 决策有效：{str(value['decision']).upper()}")
        return 0
    except GoNoGoFailure as error:
        print(f"Go/No-Go 决策无效：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
