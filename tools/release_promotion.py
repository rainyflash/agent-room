#!/usr/bin/env python3
"""维护不可跳步的 Agent Room 发布晋级记录。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
from typing import Final, Mapping, Sequence
from urllib.parse import urlparse


SCHEMA_VERSION: Final = 1
STAGES: Final = (
    "candidate",
    "database-expanded",
    "compatible-server",
    "clients-published",
    "compatibility-observed",
    "legacy-contracted",
)
SHA256_PATTERN: Final = re.compile(r"^[0-9a-f]{64}$")
REVISION_PATTERN: Final = re.compile(r"^[0-9a-f]{40}$")


class PromotionFailure(RuntimeError):
    """表示发布晋级记录无效或试图跳过门禁。"""


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    init_parser = subcommands.add_parser("init", help="创建候选发布记录")
    init_parser.add_argument("--version", required=True)
    init_parser.add_argument("--revision", required=True)
    init_parser.add_argument("--output", type=Path, required=True)

    advance_parser = subcommands.add_parser("advance", help="按顺序推进一个发布阶段")
    advance_parser.add_argument("--record", type=Path, required=True)
    advance_parser.add_argument("--output", type=Path, required=True)
    advance_parser.add_argument("--stage", choices=STAGES[1:], required=True)
    advance_parser.add_argument("--evidence-url", required=True)
    advance_parser.add_argument("--evidence-sha256", required=True)
    advance_parser.add_argument("--recorded-at-unix-seconds", type=int, required=True)

    verify_parser = subcommands.add_parser("verify", help="验证记录及指定门禁阶段")
    verify_parser.add_argument("--record", type=Path, required=True)
    verify_parser.add_argument("--expected-stage", choices=STAGES, required=True)
    verify_parser.add_argument("--version", required=True)
    verify_parser.add_argument("--revision", required=True)
    return parser.parse_args(argv)


def require_string(source: Mapping[str, object], key: str, label: str) -> str:
    value = source.get(key)
    if not isinstance(value, str) or not value:
        raise PromotionFailure(f"{label}.{key} 必须是非空字符串。")
    return value


def require_integer(source: Mapping[str, object], key: str, label: str) -> int:
    value = source.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise PromotionFailure(f"{label}.{key} 必须是非负整数。")
    return value


def validate_https(value: str, label: str) -> None:
    parsed = urlparse(value)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise PromotionFailure(f"{label} 必须是无内嵌凭据的 HTTPS 地址。")


def validate_version(value: str) -> None:
    if not re.fullmatch(
        r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:[-+][0-9A-Za-z.-]+)?",
        value,
    ):
        raise PromotionFailure("发布版本不是受支持的 SemVer。")


def load_record(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise PromotionFailure(f"发布晋级记录不是有效 JSON：{path}") from error
    if not isinstance(value, dict):
        raise PromotionFailure("发布晋级记录必须是 JSON 对象。")
    validate_record(value)
    return value


def validate_record(record: Mapping[str, object]) -> None:
    if require_integer(record, "schemaVersion", "record") != SCHEMA_VERSION:
        raise PromotionFailure("不支持的发布晋级记录版本。")
    version = require_string(record, "version", "record")
    validate_version(version)
    revision = require_string(record, "revision", "record")
    if not REVISION_PATTERN.fullmatch(revision):
        raise PromotionFailure("record.revision 必须是完整 Git SHA。")
    stage = require_string(record, "stage", "record")
    if stage not in STAGES:
        raise PromotionFailure("record.stage 不受支持。")
    history = record.get("history")
    if not isinstance(history, list):
        raise PromotionFailure("record.history 必须是数组。")
    expected_history = list(STAGES[1 : STAGES.index(stage) + 1])
    actual_history: list[str] = []
    for index, item in enumerate(history):
        label = f"record.history[{index}]"
        if not isinstance(item, dict):
            raise PromotionFailure(f"{label} 必须是对象。")
        item_stage = require_string(item, "stage", label)
        evidence_url = require_string(item, "evidenceUrl", label)
        evidence_sha256 = require_string(item, "evidenceSha256", label)
        require_integer(item, "recordedAtUnixSeconds", label)
        validate_https(evidence_url, f"{label}.evidenceUrl")
        if not SHA256_PATTERN.fullmatch(evidence_sha256):
            raise PromotionFailure(f"{label}.evidenceSha256 必须是小写 SHA-256。")
        actual_history.append(item_stage)
    if actual_history != expected_history:
        raise PromotionFailure("发布晋级历史不连续或与当前阶段不一致。")


def write_new(path: Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(value, target, ensure_ascii=False, indent=2)
            target.write("\n")
    except FileExistsError as error:
        raise PromotionFailure(f"拒绝覆盖已有发布晋级记录：{path}") from error


def initialize(version: str, revision: str, output: Path) -> None:
    validate_version(version)
    if not REVISION_PATTERN.fullmatch(revision):
        raise PromotionFailure("revision 必须是完整 Git SHA。")
    write_new(
        output,
        {
            "schemaVersion": SCHEMA_VERSION,
            "version": version,
            "revision": revision,
            "stage": "candidate",
            "history": [],
        },
    )


def advance(
    record_path: Path,
    output: Path,
    stage: str,
    evidence_url: str,
    evidence_sha256: str,
    recorded_at_unix_seconds: int,
) -> None:
    record = load_record(record_path)
    current = require_string(record, "stage", "record")
    expected = STAGES[STAGES.index(current) + 1] if current != STAGES[-1] else None
    if stage != expected:
        raise PromotionFailure(f"发布阶段不能从 {current} 跳到 {stage}；下一阶段是 {expected}。")
    validate_https(evidence_url, "evidenceUrl")
    if not SHA256_PATTERN.fullmatch(evidence_sha256):
        raise PromotionFailure("evidenceSha256 必须是小写 SHA-256。")
    if recorded_at_unix_seconds < 0:
        raise PromotionFailure("recordedAtUnixSeconds 不能为负数。")
    history = record["history"]
    if not isinstance(history, list):
        raise PromotionFailure("record.history 必须是数组。")
    updated = dict(record)
    updated["stage"] = stage
    updated["history"] = [
        *history,
        {
            "stage": stage,
            "evidenceUrl": evidence_url,
            "evidenceSha256": evidence_sha256,
            "recordedAtUnixSeconds": recorded_at_unix_seconds,
        },
    ]
    validate_record(updated)
    write_new(output, updated)


def verify(record_path: Path, expected_stage: str, version: str, revision: str) -> None:
    record = load_record(record_path)
    if record.get("stage") != expected_stage:
        raise PromotionFailure(
            f"发布晋级记录当前是 {record.get('stage')}，要求阶段为 {expected_stage}。"
        )
    if record.get("version") != version or record.get("revision") != revision:
        raise PromotionFailure("发布晋级记录与候选版本或 Git 提交不一致。")


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        if args.command == "init":
            initialize(args.version, args.revision, args.output)
        elif args.command == "advance":
            advance(
                args.record,
                args.output,
                args.stage,
                args.evidence_url,
                args.evidence_sha256,
                args.recorded_at_unix_seconds,
            )
        else:
            verify(args.record, args.expected_stage, args.version, args.revision)
        return 0
    except PromotionFailure as error:
        print(f"发布晋级失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
