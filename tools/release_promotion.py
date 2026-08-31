#!/usr/bin/env python3
"""维护不可跳步的 Agent Room 发布晋级记录。"""

from __future__ import annotations

import argparse
import hashlib
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
EVIDENCE_FILENAME_PATTERN: Final = re.compile(r"^[a-z0-9][a-z0-9._-]{0,127}\.json$")
DEPLOYMENT_EVIDENCE_KIND: Final = "agent-room.release-deployment-evidence"


class PromotionFailure(RuntimeError):
    """表示发布晋级记录无效或试图跳过门禁。"""


def parse_evidence_check(value: str) -> tuple[str, str]:
    """解析命令行中的证据检查，拒绝空名称和空详情。"""

    name, separator, detail = value.partition("=")
    if not separator or not name.strip() or not detail.strip():
        raise argparse.ArgumentTypeError("check 必须使用非空的 NAME=DETAIL 格式。")
    return name.strip(), detail.strip()


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    init_parser = subcommands.add_parser("init", help="创建候选发布记录")
    init_parser.add_argument("--version", required=True)
    init_parser.add_argument("--revision", required=True)
    init_parser.add_argument("--output", type=Path, required=True)

    create_evidence_parser = subcommands.add_parser(
        "evidence", help="生成经过 Schema 校验的部署证据"
    )
    create_evidence_parser.add_argument("--version", required=True)
    create_evidence_parser.add_argument("--revision", required=True)
    create_evidence_parser.add_argument(
        "--stage", choices=("database-expanded", "compatible-server"), required=True
    )
    create_evidence_parser.add_argument(
        "--check",
        action="append",
        type=parse_evidence_check,
        required=True,
        metavar="NAME=DETAIL",
        help="追加一个已通过的检查；可重复传入",
    )
    create_evidence_parser.add_argument(
        "--captured-at-unix-seconds", type=int, required=True
    )
    create_evidence_parser.add_argument("--output", type=Path, required=True)

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

    evidence_parser = subcommands.add_parser(
        "verify-evidence", help="把晋级记录绑定到同一 Release 的真实部署报告"
    )
    evidence_parser.add_argument("--record", type=Path, required=True)
    evidence_parser.add_argument("--root", type=Path, required=True)
    evidence_parser.add_argument("--release-base-url", required=True)
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


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


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


def create_evidence(
    version: str,
    revision: str,
    stage: str,
    checks: Sequence[tuple[str, str]],
    captured_at_unix_seconds: int,
    output: Path,
) -> None:
    """生成只能表达“已通过”事实的部署证据，避免手写 JSON 漂移。"""

    validate_version(version)
    if not REVISION_PATTERN.fullmatch(revision):
        raise PromotionFailure("revision 必须是完整 Git SHA。")
    if stage not in {"database-expanded", "compatible-server"}:
        raise PromotionFailure("部署证据阶段必须是 database-expanded 或 compatible-server。")
    if captured_at_unix_seconds < 0:
        raise PromotionFailure("capturedAtUnixSeconds 不能为负数。")
    if not checks:
        raise PromotionFailure("部署证据至少需要一个已通过检查。")

    normalized_checks: list[dict[str, object]] = []
    for name, detail in checks:
        if not name.strip() or not detail.strip():
            raise PromotionFailure("部署证据检查名称和详情不能为空。")
        normalized_checks.append(
            {
                "name": name.strip(),
                "passed": True,
                "detail": detail.strip(),
            }
        )
    write_new(
        output,
        {
            "schemaVersion": SCHEMA_VERSION,
            "kind": DEPLOYMENT_EVIDENCE_KIND,
            "version": version,
            "revision": revision,
            "capturedAtUnixSeconds": captured_at_unix_seconds,
            "result": "passed",
            "stage": stage,
            "checks": normalized_checks,
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


def validate_deployment_evidence(
    path: Path,
    *,
    stage: str,
    version: str,
    revision: str,
    recorded_at_unix_seconds: int,
) -> None:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise PromotionFailure(f"部署证据不是有效 JSON：{path.name}") from error
    if not isinstance(document, dict):
        raise PromotionFailure(f"部署证据必须是 JSON 对象：{path.name}")
    if require_integer(document, "schemaVersion", "evidence") != SCHEMA_VERSION:
        raise PromotionFailure(f"部署证据版本不受支持：{path.name}")
    if require_string(document, "kind", "evidence") != DEPLOYMENT_EVIDENCE_KIND:
        raise PromotionFailure(f"部署证据 kind 不受支持：{path.name}")
    if require_string(document, "stage", "evidence") != stage:
        raise PromotionFailure(f"部署证据阶段与晋级记录不一致：{path.name}")
    if require_string(document, "version", "evidence") != version:
        raise PromotionFailure(f"部署证据版本与晋级记录不一致：{path.name}")
    if require_string(document, "revision", "evidence") != revision:
        raise PromotionFailure(f"部署证据提交与晋级记录不一致：{path.name}")
    captured_at = require_integer(document, "capturedAtUnixSeconds", "evidence")
    if captured_at > recorded_at_unix_seconds:
        raise PromotionFailure(f"部署证据时间晚于晋级记录：{path.name}")
    if require_string(document, "result", "evidence") != "passed":
        raise PromotionFailure(f"部署证据没有通过：{path.name}")
    checks = document.get("checks")
    if not isinstance(checks, list) or not checks:
        raise PromotionFailure(f"部署证据 checks 必须是非空数组：{path.name}")
    for index, check in enumerate(checks):
        label = f"evidence.checks[{index}]"
        if not isinstance(check, dict):
            raise PromotionFailure(f"{label} 必须是对象。")
        require_string(check, "name", label)
        require_string(check, "detail", label)
        if check.get("passed") is not True:
            raise PromotionFailure(f"{label}.passed 必须为 true。")


def verify_evidence(record_path: Path, root: Path, release_base_url: str) -> None:
    record = load_record(record_path)
    validate_https(release_base_url, "releaseBaseUrl")
    parsed_base = urlparse(release_base_url)
    if not release_base_url.endswith("/") or parsed_base.query or parsed_base.fragment:
        raise PromotionFailure("releaseBaseUrl 必须以 / 结尾且不能包含查询或片段。")
    try:
        resolved_root = root.resolve(strict=True)
    except OSError as error:
        raise PromotionFailure(f"候选证据目录不存在：{root}") from error
    history = record.get("history")
    if not isinstance(history, list):
        raise PromotionFailure("record.history 必须是数组。")
    version = require_string(record, "version", "record")
    revision = require_string(record, "revision", "record")
    for index, item in enumerate(history):
        if not isinstance(item, dict):
            raise PromotionFailure(f"record.history[{index}] 必须是对象。")
        label = f"record.history[{index}]"
        stage = require_string(item, "stage", label)
        evidence_url = require_string(item, "evidenceUrl", label)
        expected_digest = require_string(item, "evidenceSha256", label)
        recorded_at = require_integer(item, "recordedAtUnixSeconds", label)
        if not evidence_url.startswith(release_base_url):
            raise PromotionFailure(f"{label}.evidenceUrl 不属于当前 Release。")
        filename = evidence_url.removeprefix(release_base_url)
        if not EVIDENCE_FILENAME_PATTERN.fullmatch(filename):
            raise PromotionFailure(f"{label}.evidenceUrl 不是受约束的 JSON 资产。")
        try:
            evidence_path = (resolved_root / filename).resolve(strict=True)
            evidence_path.relative_to(resolved_root)
        except (OSError, ValueError) as error:
            raise PromotionFailure(f"部署证据资产不存在或逃逸候选目录：{filename}") from error
        if not evidence_path.is_file() or sha256_file(evidence_path) != expected_digest:
            raise PromotionFailure(f"部署证据摘要不匹配：{filename}")
        validate_deployment_evidence(
            evidence_path,
            stage=stage,
            version=version,
            revision=revision,
            recorded_at_unix_seconds=recorded_at,
        )


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        if args.command == "init":
            initialize(args.version, args.revision, args.output)
        elif args.command == "evidence":
            create_evidence(
                args.version,
                args.revision,
                args.stage,
                args.check,
                args.captured_at_unix_seconds,
                args.output,
            )
        elif args.command == "advance":
            advance(
                args.record,
                args.output,
                args.stage,
                args.evidence_url,
                args.evidence_sha256,
                args.recorded_at_unix_seconds,
            )
        elif args.command == "verify":
            verify(args.record, args.expected_stage, args.version, args.revision)
        else:
            verify_evidence(args.record, args.root, args.release_base_url)
        return 0
    except PromotionFailure as error:
        print(f"发布晋级失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
