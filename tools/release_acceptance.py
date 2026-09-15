#!/usr/bin/env python3
"""Validate same-candidate usability evidence before public release.

Reports come from actual acceptance runs, not from the release scheduler. Missing
evidence pauses a release; fixture-based checks cannot substitute for a host turn.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import time
import json
import shutil
import tempfile

try:
    from .release import ReleaseFailure, load_object, resolve_local_file, sha256_file
except ImportError:
    from release import ReleaseFailure, load_object, resolve_local_file, sha256_file


SCENARIOS = {
    "first-device": {"authorizationCompleted", "bridgeConnected", "agentJoined", "replyVerified"},
    "upgrade": {"installedPreviousVersion", "upgradedToCandidate", "loginRestored", "identityPreserved", "pendingDeliveryPreserved"},
    "continuous-reception": {"realHostInvoked", "twoIncomingMessages", "twoRepliesVerified", "idleDidNotInvokeHost", "manualTakeoverStoppedReceiver", "resumeKeptCursor"},
}


def verify(root: Path, version: str, revision: str, *, now: int | None = None) -> None:
    index = load_object(root / "release-usability-acceptance.json", "易用性验收清单")
    metadata = load_object(root / "release-metadata.json", "候选元数据")
    if index.get("schemaVersion") != 1 or index.get("version") != version or index.get("revision") != revision:
        raise ReleaseFailure("易用性验收必须来自当前版本和提交。")
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or metadata.get("revision") != revision or metadata.get("version") != version:
        raise ReleaseFailure("候选和易用性验收修订不一致。")
    digest = sha256_file(root / "release.signed.json")
    if index.get("signedManifestSha256") != digest:
        raise ReleaseFailure("易用性验收没有绑定本次签名候选。")
    reports = index.get("reports")
    if not isinstance(reports, dict) or set(reports) != set(SCENARIOS):
        raise ReleaseFailure("必须提交新电脑接入、原版本升级、真实宿主持续接待三份报告。")
    current = int(time.time()) if now is None else now
    for scenario, required in SCENARIOS.items():
        reference = reports[scenario]
        if not isinstance(reference, dict) or not isinstance(reference.get("path"), str) or not re.fullmatch(r"[A-Za-z0-9_.-]+\.json", reference["path"]):
            raise ReleaseFailure(f"{scenario} 报告引用无效。")
        path = resolve_local_file(root, reference["path"], scenario)
        if sha256_file(path) != reference.get("sha256"):
            raise ReleaseFailure(f"{scenario} 报告摘要不符。")
        report = load_object(path, scenario)
        if any(report.get(key) != value for key, value in {
            "schemaVersion": 1, "scenario": scenario, "version": version,
            "revision": revision, "signedManifestSha256": digest, "result": "passed",
        }.items()):
            raise ReleaseFailure(f"{scenario} 报告未通过或不是当前候选。")
        captured = report.get("capturedAtUnixSeconds")
        published = metadata.get("publishedAtUnixSeconds")
        if not isinstance(captured, int) or isinstance(captured, bool) or not isinstance(published, int) or not published <= captured <= current + 60:
            raise ReleaseFailure(f"{scenario} 报告时间不在候选验收期内。")
        checks = report.get("checks")
        if not isinstance(checks, dict) or not required <= set(checks) or any(checks.get(key) is not True for key in required):
            raise ReleaseFailure(f"{scenario} 缺少已通过的必需检查。")
        if report.get("fixture") is not False:
            raise ReleaseFailure(f"{scenario} 必须是实际运行验收，不能用夹具代替。")
        evidence = report.get("evidence")
        if not isinstance(evidence, list) or not evidence:
            raise ReleaseFailure(f"{scenario} 缺少原始脱敏验收记录。")
        for item in evidence:
            if not isinstance(item, dict) or not isinstance(item.get("path"), str) or not re.fullmatch(r"usability-evidence-[A-Za-z0-9_.-]+", item["path"]) or item.get("redacted") is not True:
                raise ReleaseFailure(f"{scenario} 验收记录引用无效。")
            if sha256_file(resolve_local_file(root, item["path"], scenario)) != item.get("sha256"):
                raise ReleaseFailure(f"{scenario} 原始验收记录摘要不符。")
        if scenario == "continuous-reception":
            receipts = report.get("replyReceipts")
            if not isinstance(receipts, list) or len(receipts) < 2:
                raise ReleaseFailure("持续接待缺少两条实际回复回执。")
            for receipt in receipts:
                if not isinstance(receipt, dict) or not isinstance(receipt.get("eventId"), str) or not receipt["eventId"].startswith("$") or not re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}", str(receipt.get("submissionId"))):
                    raise ReleaseFailure("持续接待回复回执无效。")
            if len({receipt["eventId"] for receipt in receipts}) != len(receipts) or len({receipt["submissionId"] for receipt in receipts}) != len(receipts):
                raise ReleaseFailure("持续接待不能把同一回复计算两次。")


def assemble(root: Path, reports: list[Path]) -> None:
    """Import real-run reports and compute their inventory; never invent checks."""
    metadata = load_object(root / "release-metadata.json", "metadata")
    version, revision = metadata.get("version"), metadata.get("revision")
    if not isinstance(version, str) or not isinstance(revision, str):
        raise ReleaseFailure("候选身份无效。")
    with tempfile.TemporaryDirectory(prefix="agent-room-usability-") as temporary:
        staging = Path(temporary)
        for name in ("release.signed.json", "release-metadata.json"):
            shutil.copyfile(root / name, staging / name)
        references: dict[str, object] = {}
        for path in reports:
            report = load_object(path, "实际验收报告")
            scenario = report.get("scenario")
            if not isinstance(scenario, str) or scenario not in SCENARIOS or scenario in references:
                raise ReleaseFailure("验收场景缺失或重复。")
            destination = staging / f"release-usability-{scenario}.json"
            shutil.copyfile(path, destination)
            references[scenario] = {"path": destination.name, "sha256": sha256_file(destination)}
            evidence = report.get("evidence")
            if not isinstance(evidence, list):
                raise ReleaseFailure("报告没有脱敏原始记录。")
            for item in evidence:
                name = item.get("path") if isinstance(item, dict) else None
                if not isinstance(name, str) or not re.fullmatch(r"usability-evidence-[A-Za-z0-9_.-]+", name):
                    raise ReleaseFailure("原始记录文件名必须以 usability-evidence- 开头，只能包含字母数字下划线点或短横线。")
                source = resolve_local_file(path.parent, name, "原始验收记录")
                existing = staging / name
                if existing.exists() and sha256_file(existing) != sha256_file(source):
                    raise ReleaseFailure("不同报告引用了同名但不同的验收记录。")
                shutil.copyfile(source, existing)
        index = {"schemaVersion": 1, "version": version, "revision": revision, "signedManifestSha256": sha256_file(root / "release.signed.json"), "reports": references}
        (staging / "release-usability-acceptance.json").write_text(json.dumps(index, indent=2) + "\n", encoding="utf-8")
        verify(staging, version, revision)
        paths = [path for path in staging.iterdir() if path.name not in {"release.signed.json", "release-metadata.json"}]
        for path in paths:
            target = root / path.name
            if target.exists() and sha256_file(target) != sha256_file(path):
                raise ReleaseFailure(f"拒绝覆盖已记录的不同验收内容：{path.name}")
        for path in paths:
            shutil.copyfile(path, root / path.name)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--version")
    parser.add_argument("--revision")
    parser.add_argument("--assemble", nargs=3, type=Path, metavar="REAL_REPORT")
    args = parser.parse_args()
    try:
        if args.assemble:
            assemble(args.root, args.assemble)
        else:
            if not args.version or not args.revision:
                raise ReleaseFailure("验证时必须指定 --version 和 --revision。")
            verify(args.root, args.version, args.revision)
    except (ReleaseFailure, OSError) as error:
        parser.exit(1, f"易用性验收未通过：{error}\n")
    print("新设备、升级和真实持续接待验收均匹配当前候选。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
