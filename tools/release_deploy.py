#!/usr/bin/env python3
"""Deploy the verified candidate with resumable backup/migration/server/web stages.

Called by release_flow on the Linux deployment host. Existing configuration and
secrets are preserved; only application image references change.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import release, release_promotion
from tools.release_flow import atomic_json, command, exclusive
from tools.prodops.config import load_deployment_config
from tools.prodops.render import DeploymentPaths
from tools.prodops.runtime import ProductionRuntime


def image_overlay(images: dict[str, str]) -> dict[str, object]:
    if set(images) != {"control-plane", "identity", "web"}:
        raise RuntimeError("完整候选必须包含三套应用镜像。")
    for name, reference in images.items():
        if not re.fullmatch(rf"ghcr\.io/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/{name}@sha256:[0-9a-f]{{64}}", reference):
            raise RuntimeError("候选必须使用完整的镜像摘要。")
    return {"services": {name: {"image": reference} for name, reference in {
        "control-plane": images["control-plane"], "identity": images["identity"],
        "gateway": images["web"], "migrate": images["control-plane"], "object-store-init": images["control-plane"],
    }.items()}}


class Deployment:
    def __init__(self, args: argparse.Namespace):
        if not sys.platform.startswith("linux"):
            raise RuntimeError("生产部署命令应在 Linux 部署主机运行；Windows 可准备候选和验收报告。")
        self.args = args
        self.candidate = args.candidate.resolve(strict=True)
        self.metadata = release.load_object(self.candidate / "release-metadata.json", "候选元数据")
        if self.metadata.get("revision") != args.revision or release.sha256_file(self.candidate / "release.signed.json") != args.manifest_sha256:
            raise RuntimeError("部署输入与已核验的候选不符。")
        release.validate_candidate_files(self.candidate, self.candidate / "release-inventory.json", release.CandidatePaths(self.candidate / "release.json", self.candidate / "release-evidence.json"))
        inventory = release.load_object(self.candidate / "release-inventory.json", "inventory")
        self.images = {item.name: item.url.removeprefix("oci://") for item in release.parse_artifacts(self.candidate, inventory) if item.kind == "oci-image"}
        self.overlay = image_overlay(self.images)
        self.runtime = ProductionRuntime(load_deployment_config(args.config), DeploymentPaths.from_state(args.state_dir))
        self.work = args.state_dir / "releases" / args.manifest_sha256
        self.work.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.checkpoint = self.work / "deployment.json"
        self.state = dict(release.load_object(self.checkpoint, "部署检查点")) if self.checkpoint.exists() else {}
        if self.state and (self.state.get("revision") != args.revision or self.state.get("manifest") != args.manifest_sha256 or self.state.get("images") != self.images):
            raise RuntimeError("部署检查点属于另一个候选。")

    def configuration_digest(self) -> str:
        paths = [self.args.config, self.runtime.paths.compose_environment]
        for directory in (self.runtime.paths.generated, self.runtime.paths.secrets):
            paths.extend(sorted(path for path in directory.rglob("*") if path.is_file()))
        manifest = {str(path): release.sha256_file(path) for path in paths}
        return hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()

    def save(self) -> None:
        atomic_json(self.checkpoint, self.state)
        self.checkpoint.chmod(0o600)

    def run(self, *arguments: str) -> str:
        return command(list(arguments))

    def compose(self, *arguments: str) -> None:
        self.run(*self.runtime.compose_command(), "--file", str(self.work / "compose.images.json"), *arguments)

    def services(self) -> dict[str, dict[str, object]]:
        ids = self.run("docker", "ps", "--filter", f"label=com.docker.compose.project={self.runtime.config.project_name}", "--format", "{{.ID}}").split()
        if not ids:
            raise RuntimeError("找不到已运行的部署，本命令只负责升级。")
        # Docker inspect includes environment values. Extract only identities;
        # never persist or print the raw response.
        raw = json.loads(self.run("docker", "inspect", *ids))
        result: dict[str, dict[str, object]] = {}
        for row in raw:
            name = row["Config"]["Labels"]["com.docker.compose.service"]
            if name in result:
                raise RuntimeError("本部署入口要求每个服务只有一个副本。")
            result[name] = {"containerId": row["Id"], "imageId": row["Image"], "image": row["Config"]["Image"], "health": row["State"].get("Health", {}).get("Status")}
        return result

    def baseline(self) -> None:
        if self.run("git", "rev-parse", "HEAD").strip() != self.args.revision or self.run("git", "status", "--porcelain", "--untracked-files=no").strip():
            raise RuntimeError("部署工具必须来自已锁定提交的干净工作树。")
        if self.state and self.state.get("configurationSha256") != self.configuration_digest():
            raise RuntimeError("部署期间配置或密钥变化，拒绝继续。")

    def prepare(self) -> None:
        self.baseline()
        if not self.state:
            self.runtime.preflight(require_linux=True, require_dns=True, require_ports=False)
            self.runtime.health(timeout_seconds=45)
            self.runtime.federation(timeout_seconds=15)
            before = self.services()
            if not {"postgres", "synapse", "object-store", "identity", "control-plane", "gateway"} <= set(before):
                raise RuntimeError("现网服务不完整。")
            # The backup id is recorded immediately after successful creation.
            # If interrupted before this checkpoint, another backup is safe.
            configuration = self.configuration_digest()
            backup = self.runtime.backup(preserve_configuration=True)
            self.runtime.verify_backup(backup.backup_id)
            if configuration != self.configuration_digest():
                raise RuntimeError("备份准备改变了现有配置；已保留备份，停止部署以核对差异。")
            self.state = {"revision": self.args.revision, "manifest": self.args.manifest_sha256,
                          "images": self.images, "configurationSha256": self.configuration_digest(),
                          "backupId": backup.backup_id, "before": before}
            self.save()
        backup_id = self.state.get("backupId")
        if not isinstance(backup_id, str):
            raise RuntimeError("没有可验证的升级前备份。")
        self.runtime.verify_backup(backup_id)
        overlay_path = self.work / "compose.images.json"
        if overlay_path.exists() and release.load_object(overlay_path, "镜像覆盖配置") != self.overlay:
            raise RuntimeError("镜像覆盖配置已经变化。")
        atomic_json(overlay_path, self.overlay)
        before = self.state["before"]
        atomic_json(self.work / "compose.rollback.json", {"services": {name: {"image": before[name]["imageId"]} for name in ("identity", "control-plane", "gateway")}})
        self.compose("config", "--quiet")
        if not self.state.get("pulled"):
            for reference in self.images.values():
                self.run("docker", "pull", reference)
            self.state["pulled"] = True
            self.save()

    def evidence(self, stage: str, checks: list[tuple[str, str]]) -> None:
        version, revision = str(self.metadata["version"]), self.args.revision
        report = self.candidate / f"release-deployment-{stage}.json"
        record = self.candidate / f"release-promotion-{stage}.json"
        previous = self.candidate / ("release-promotion.json" if stage == "database-expanded" else "release-promotion-database-expanded.json")
        now = int(time.time())
        if not report.exists():
            release_promotion.create_evidence(version, revision, stage, checks, now, report)
        report_data = release.load_object(report, "部署报告")
        captured = report_data.get("capturedAtUnixSeconds")
        if not isinstance(captured, int):
            raise RuntimeError("部署报告时间无效。")
        release_promotion.validate_deployment_evidence(report, stage=stage, version=version, revision=revision, recorded_at_unix_seconds=now)
        if not record.exists():
            release_promotion.advance(previous, record, stage, f"{self.metadata['releaseBaseUrl']}/{report.name}", release.sha256_file(report), now)
        release_promotion.verify(record, stage, version, revision)
        release_promotion.verify_evidence(record, self.candidate, f"{self.metadata['releaseBaseUrl']}/")

    def verify_running(self, *, web: bool) -> None:
        after = self.services()
        expected = {"control-plane": self.images["control-plane"], "identity": self.images["identity"]}
        if web:
            expected["gateway"] = self.images["web"]
        for service, reference in expected.items():
            image = json.loads(self.run("docker", "image", "inspect", reference))[0]["Id"]
            if after.get(service, {}).get("image") != reference or after[service]["imageId"] != image or after[service]["health"] != "healthy":
                raise RuntimeError(f"{service} 未运行预期的健康候选镜像。")
        for service, previous in self.state["before"].items():
            if service not in expected and after.get(service, {}).get("containerId") != previous["containerId"]:
                raise RuntimeError(f"升级期间 {service} 意外发生变化。")
        self.runtime.health(timeout_seconds=45)
        self.runtime.federation(timeout_seconds=15)
        self.baseline()

    def server(self) -> None:
        self.prepare()
        if not self.state.get("migrated"):
            # SQLx migrations are transactional and track applied versions. It is
            # safe to repeat this command after an uncertain process result.
            self.compose("run", "--rm", "--no-deps", "--pull", "never", "migrate")
            self.baseline()
            self.state["migrated"] = True
            self.save()
        self.evidence("database-expanded", [("candidate-migrations", self.images["control-plane"]), ("verified-backup", str(self.state["backupId"]))])
        if not self.state.get("server"):
            self.compose("up", "-d", "--no-deps", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "180", "identity", "control-plane")
        if not self.state.get("identityReconciled"):
            # Realm settings (theme, languages, registration) follow each release, not only fresh installs.
            # The script closes registration first, so a failure here leaves it closed until a retry succeeds.
            registration = self.runtime.config.identity.registration
            service = "identity-registration-open" if registration.is_open else "identity-registration-close"
            self.compose("run", "--rm", "--no-deps", "--pull", "never", service)
            self.state["identityReconciled"] = True
            self.save()
        self.verify_running(web=bool(self.state.get("web")))
        self.state["server"] = True
        self.save()
        self.evidence("compatible-server", [("health-and-federation", "Passed against running candidate services"), ("preserved-configuration", self.configuration_digest()), ("candidate-manifest", self.args.manifest_sha256)])

    def web(self) -> None:
        if not self.state.get("server"):
            raise RuntimeError("网页部署前必须完成并验证服务端兼容部署。")
        self.baseline()
        # The orchestrator verifies published assets and channel before this call.
        # The immutable server record still has to match the candidate here.
        release_promotion.verify(self.candidate / "release-promotion-compatible-server.json", "compatible-server", str(self.metadata["version"]), self.args.revision)
        if not self.state.get("web"):
            self.compose("up", "-d", "--no-deps", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "90", "gateway")
        self.verify_running(web=True)
        self.state["web"] = True
        self.save()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("server", "web"))
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    args = parser.parse_args()
    try:
        if not re.fullmatch(r"[0-9a-f]{64}", args.manifest_sha256) or not re.fullmatch(r"[0-9a-f]{40}", args.revision):
            raise RuntimeError("发布候选标识无效。")
        with exclusive(args.state_dir / "releases" / "deployment.lock"):
            deployment = Deployment(args)
            getattr(deployment, args.phase)()
        print(f"候选 {args.phase} 部署已核验。")
        return 0
    except (RuntimeError, ValueError, OSError) as error:
        parser.exit(1, f"部署停止，备份与检查点保留：{error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
