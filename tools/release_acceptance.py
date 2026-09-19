#!/usr/bin/env python3
"""Validate same-candidate usability evidence before public release.

Reports come from actual acceptance runs, not from the release scheduler. Missing
evidence pauses a release; fixture-based checks cannot substitute for a host turn.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import subprocess
import time
import json
import shutil
import tempfile
from typing import Callable

try:
    from .release import ReleaseFailure, load_object, resolve_local_file, sha256_file
except ImportError:
    from release import ReleaseFailure, load_object, resolve_local_file, sha256_file


SCENARIOS = {
    "first-device": {"authorizationCompleted", "bridgeConnected", "agentJoined", "replyVerified"},
    "upgrade": {"installedPreviousVersion", "upgradedToCandidate", "loginRestored", "identityPreserved", "pendingDeliveryPreserved"},
    "continuous-reception": {"realHostInvoked", "twoIncomingMessages", "twoRepliesVerified", "idleDidNotInvokeHost", "manualTakeoverStoppedReceiver", "resumeKeptCursor"},
}
# A release may run first-device on the long-lived acceptance device instead of a fresh profile. It then
# proves the candidate Bridge restores that device's saved authorization, and is only accepted while the
# device's last fresh authorization is recent and no login code has changed since.
REUSED_DEVICE_CHECKS = {"authorizationRestored", "bridgeConnected", "agentJoined", "replyVerified"}
FRESH_AUTHORIZATION_MAX_AGE_SECONDS = 30 * 24 * 60 * 60
LOGIN_PATHS = (
    "apps/bridge/src/config.rs",
    "apps/control-plane/src/features/authentication.rs",
    "apps/control-plane/src/features/devices.rs",
    "crates/application/src/devices.rs",
    "crates/application/src/ports/identity.rs",
    "crates/bridge-core/src/authorization.rs",
    "crates/identity-adapter/",
    "crates/postgres-adapter/src/devices.rs",
    # The identity theme renders the sign-in and device approval pages; the image pins Keycloak itself.
    "infra/identity/",
    "infra/oidc/",
    "infra/production/Containerfile.identity",
    "infra/production/keycloak-registration-reconcile.py",
    "tools/prodops/render.py",
)
LoginChanges = Callable[[str, str], list[str]]
# A release that moves the product to another server cannot carry the old server's login, identity or
# deliveries across. Each such release is listed with its move. Its upgrade report still proves the in-place
# upgrade, and instead proves the upgraded app did not reuse the old login and signs in to the new server.
SERVER_MIGRATIONS = {"0.1.0-alpha.43": {"from": "room.the-zeroth.com", "to": "agentroom.chat"}}
MIGRATED_UPGRADE_CHECKS = {"installedPreviousVersion", "upgradedToCandidate", "previousLoginNotReused",
                           "signedInToNewServer"}
PRESERVATION_CHECKS = {"loginRestored", "identityPreserved", "pendingDeliveryPreserved"}


def login_changes(base: str, head: str) -> list[str]:
    """Login-related paths changed between two revisions; fails closed when history is unavailable."""
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(["git", "-C", str(root), "diff", "--name-only", base, head],
                            capture_output=True, text=True, encoding="utf-8", check=False)
    if result.returncode:
        raise ReleaseFailure("无法比对上次新设备授权之后的代码变化。")
    return [path for path in result.stdout.splitlines() if path.startswith(LOGIN_PATHS)]


def reuse_blocker(fresh: object, revision: str, now: int, changes: LoginChanges = login_changes) -> str | None:
    """Why this revision may not reuse the long-lived acceptance device, or None when it may."""
    if (not isinstance(fresh, dict) or not isinstance(fresh.get("version"), str)
            or not re.fullmatch(r"[0-9a-f]{40}", str(fresh.get("revision")))):
        return "长期验收设备缺少有效的新设备授权记录。"
    captured = fresh.get("capturedAtUnixSeconds")
    if not isinstance(captured, int) or isinstance(captured, bool) or captured > now:
        return "长期验收设备的授权时间无效。"
    if now - captured > FRESH_AUTHORIZATION_MAX_AGE_SECONDS:
        return "上次新设备授权已超过 30 天。"
    changed = changes(fresh["revision"], revision)
    if changed:
        return "上次新设备授权之后登录相关代码有变化：" + "、".join(changed[:5])
    return None


def verify(root: Path, version: str, revision: str, *, now: int | None = None,
           changes: LoginChanges = login_changes) -> None:
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
        if scenario == "first-device":
            mode = report.get("deviceMode", "fresh")
            if mode == "reused":
                blocker = reuse_blocker(report.get("freshAuthorization"), revision, current, changes)
                if blocker is not None:
                    raise ReleaseFailure(f"first-device 不能复用长期验收设备：{blocker}")
                if isinstance(checks, dict) and "authorizationCompleted" in checks:
                    raise ReleaseFailure("复用长期验收设备时不能声称本次完成了设备授权。")
                required = REUSED_DEVICE_CHECKS
            elif mode != "fresh":
                raise ReleaseFailure("first-device 设备模式无效。")
        if scenario == "upgrade":
            mode = report.get("upgradeMode", "in-place")
            if mode == "server-migration":
                migration = SERVER_MIGRATIONS.get(version)
                if migration is None or report.get("serverMigration") != migration:
                    raise ReleaseFailure("只有登记过的服务器迁移版本，升级验收才可以不保留旧服务器上的登录和数据。")
                if isinstance(checks, dict) and PRESERVATION_CHECKS & set(checks):
                    raise ReleaseFailure("服务器迁移的升级报告不能声称保留了旧服务器上的登录或数据。")
                required = MIGRATED_UPGRADE_CHECKS
            elif mode != "in-place":
                raise ReleaseFailure("upgrade 升级模式无效。")
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
