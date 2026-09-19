#!/usr/bin/env python3
"""One resumable release command; checkpoints never replace live release gates."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid
from collections.abc import Iterator
from collections.abc import Callable
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import release, release_acceptance, release_promotion, release_native
from tools.release_ci import validate_repository, workspace_version

CI_JOBS = frozenset({
    "格式、类型与测试", "Windows 客户端运行时原生检查", "Web 真实浏览器验收",
    "供应链与物料清单", "真实网页登录与会话恢复", "PostgreSQL、Matrix、对象存储与协议集成",
    "Linux Agent 运行时镜像与 HTTPS MCP 验收", "Linux 无桌面凭据恢复与 Matrix 真实收发",
})
PUBLISH_WORKFLOW = "release-publish.yml"


class Waiting(RuntimeError):
    """The same command can resume after an external run or evidence completes."""

    def __init__(self, message: str, *, pollable: bool = False):
        super().__init__(message)
        self.pollable = pollable


def atomic_json(path: Path, value: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temporary.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(value, target, ensure_ascii=False, sort_keys=True, indent=2)
            target.write("\n")
            target.flush()
            os.fsync(target.fileno())
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


@contextmanager
def exclusive(path: Path) -> Iterator[None]:
    """OS releases the lock on crash. Never delete another process's lock file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as handle:
        if handle.tell() == 0:
            handle.write(b"0")
            handle.flush()
        handle.seek(0)
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            yield
        finally:
            handle.seek(0)
            if os.name == "nt":
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle, fcntl.LOCK_UN)


def command(args: list[str], *, capture: bool = True) -> str:
    result = subprocess.run(args, cwd=ROOT, text=True, encoding="utf-8", capture_output=capture, check=False,
                            **({"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}))
    if result.returncode:
        # Do not print subprocess stderr: deployment tools may include secrets.
        raise RuntimeError(f"{Path(args[0]).name} 执行失败（{result.returncode}），该阶段没有完成。")
    return result.stdout if capture else ""


class GitHub:
    def __init__(self, repository: str):
        validate_repository(repository)
        self.repository = repository

    def call(self, *args: str) -> str:
        return command(["gh", *args, "--repo", self.repository])

    def json(self, *args: str) -> object:
        return json.loads(self.call(*args))

    def main_revision(self) -> str:
        value = json.loads(command(["gh", "api", f"repos/{self.repository}/commits/main"]))
        revision = value.get("sha") if isinstance(value, dict) else None
        if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
            raise RuntimeError("无法核对受保护主分支。")
        return revision

    def on_main(self, revision: str) -> bool:
        value = json.loads(command(["gh", "api", f"repos/{self.repository}/compare/{revision}...main"]))
        return isinstance(value, dict) and value.get("status") in {"ahead", "identical"}

    def branch_revision(self, branch: str) -> str | None:
        refs = json.loads(command(["gh", "api", f"repos/{self.repository}/git/matching-refs/heads/{branch}"]))
        if not isinstance(refs, list):
            raise RuntimeError("无法核对发布分支。")
        # matching-refs 按前缀匹配，release/v1.0 也会列出 release/v1.0.1。
        exact = [ref for ref in refs if isinstance(ref, dict) and ref.get("ref") == f"refs/heads/{branch}"]
        if not exact:
            return None
        target = exact[0].get("object")
        revision = target.get("sha") if isinstance(target, dict) else None
        if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
            raise RuntimeError("无法核对发布分支。")
        return revision

    def create_branch(self, branch: str, revision: str) -> None:
        command(["gh", "api", "-X", "POST", f"repos/{self.repository}/git/refs",
                 "-f", f"ref=refs/heads/{branch}", "-f", f"sha={revision}"])

    def branch_protected(self, branch: str) -> bool:
        value = json.loads(command(["gh", "api", f"repos/{self.repository}/branches/{quote(branch, safe='')}"]))
        return isinstance(value, dict) and value.get("protected") is True


def completed_run(run: dict[str, object], revision: str, required_jobs: frozenset[str] = frozenset(),
                  branch: str = "main") -> None:
    if run.get("headSha") != revision or run.get("headBranch") != branch:
        raise RuntimeError(f"远端运行不属于已锁定的 {branch} 提交。")
    if run.get("status") != "completed":
        raise Waiting(f"远端运行尚未结束：{run.get('url', '')}", pollable=True)
    if run.get("conclusion") != "success":
        raise Waiting(f"远端运行未通过，请修复或重跑该运行，随后继续同一命令：{run.get('url', '')}")
    jobs = run.get("jobs")
    if not isinstance(jobs, list):
        raise RuntimeError("远端运行缺少任务结果。")
    successful = {job.get("name") for job in jobs if isinstance(job, dict) and job.get("conclusion") == "success"}
    if not required_jobs <= successful:
        raise RuntimeError("远端运行跳过了必须的完整检查。")


class ReleaseFlow:
    def __init__(self, path: Path, state: dict[str, object], github: GitHub):
        self.path, self.state, self.github = path, state, github
        self.directory = path.parent
        self.candidate = self.directory / "candidate"
        # Recheck every gate on a new invocation, but do not re-verify gigabytes
        # of unchanged artifacts every time --watch polls a running workflow.
        self.completed: set[str] = set()

    def value(self, key: str) -> str:
        value = self.state.get(key)
        if not isinstance(value, str) or not value:
            raise RuntimeError(f"发布检查点缺少 {key}。")
        return value

    def save(self) -> None:
        atomic_json(self.path, self.state)

    def stage(self, name: str) -> None:
        if self.state.get("stage") != name:
            self.state["stage"] = name
            self.save()
            print(f"发布进度：{name}", flush=True)

    def workflow(self, workflow: str, fields: dict[str, str], *, required: frozenset[str] = frozenset()) -> None:
        runs = self.state.setdefault("runs", {})
        if not isinstance(runs, dict):
            raise RuntimeError("运行检查点无效。")
        entry = runs.get(workflow)
        if entry is None:
            entry = {"operation": uuid.uuid4().hex, "dispatched": False}
            runs[workflow] = entry
            self.save()
        if not isinstance(entry, dict) or not isinstance(entry.get("operation"), str):
            raise RuntimeError("运行身份检查点无效。")
        title = f"release-flow:{entry['operation']}"
        revision = self.value("revision")
        branch = self.dispatch_branch(workflow)
        if "id" not in entry:
            found = self.github.json("run", "list", "--workflow", workflow, "--branch", branch, "--commit", revision,
                                     "--event", "workflow_dispatch", "--limit", "100", "--json", "databaseId,displayTitle,headSha")
            if not isinstance(found, list):
                raise RuntimeError("无法查找远端运行。")
            matched = [row for row in found if isinstance(row, dict) and row.get("displayTitle") == title and row.get("headSha") == revision]
            if len(matched) > 1:
                raise RuntimeError("同一发布操作发现重复运行，拒绝猜测使用哪一个。")
            if matched:
                entry["id"] = matched[0]["databaseId"]
                self.save()
            elif entry.get("dispatched"):
                # A timeout may occur after GitHub accepted the request. Do not
                # resubmit and create another build/signature on an uncertain result.
                raise Waiting(f"正在核对 {workflow} 的派发结果；不会重复创建候选。操作号 {entry['operation']}。可用 --attach-run 工作流=编号核对已存在的运行。")
            else:
                if workflow == PUBLISH_WORKFLOW:
                    self.pin_release_branch(branch)
                elif self.github.main_revision() != revision:
                    raise RuntimeError("main 已变化，请为新的提交创建独立发布检查点。")
                entry["dispatched"] = True
                self.save()  # durable intent precedes the mutation
                arguments = ["workflow", "run", workflow, "--ref", branch]
                for key, value in {**fields, "operation_id": entry["operation"], "expected_revision": revision}.items():
                    arguments.extend(("-f", f"{key}={value}"))
                self.github.call(*arguments)
                raise Waiting(f"已启动 {workflow}，再次运行同一命令可继续。", pollable=True)
        identifier = entry.get("id")
        if not isinstance(identifier, int) or isinstance(identifier, bool) or identifier <= 0:
            raise RuntimeError("远端运行编号无效。")
        result = self.github.json("run", "view", str(identifier), "--json", "headSha,headBranch,status,conclusion,jobs,url")
        if not isinstance(result, dict):
            raise RuntimeError("远端运行结果无效。")
        try:
            completed_run(result, revision, required, branch)
        except Waiting:
            if workflow != "release-candidate.yml" or result.get("status") != "completed" or result.get("conclusion") == "success":
                raise
            if self.state.get("recoveredCandidateRun") == identifier:
                # The archive may have expired and the draft may already be public.
                # Revalidate the exact saved candidate without repeating its upload.
                self.verify_candidate()
            else:
                self.recover_candidate(identifier)

    def dispatch_branch(self, workflow: str) -> str:
        # CI and the candidate build on main right after the version merge; publication runs
        # from a protected branch pinned to the candidate, so main can keep merging meanwhile.
        return f"release/{self.value('tag')}" if workflow == PUBLISH_WORKFLOW else "main"

    def pin_release_branch(self, branch: str) -> None:
        revision = self.value("revision")
        if not self.github.on_main(revision):
            raise RuntimeError("候选提交不在受保护的 main 上，不能发布。")
        current = self.github.branch_revision(branch)
        if current is None:
            self.github.create_branch(branch, revision)
        elif current != revision:
            raise RuntimeError(f"{branch} 没有指向锁定提交；不会移动已有的发布分支。")
        if not self.github.branch_protected(branch):
            raise RuntimeError(f"{branch} 未受保护，public-release 环境会拒绝它；请先为 release/* 设置分支保护。")

    def attach_run(self, specification: str) -> None:
        workflow, separator, identifier = specification.partition("=")
        if not separator or workflow not in {"ci.yml", "release-candidate.yml", "release-publish.yml"} or not re.fullmatch(r"[1-9][0-9]*", identifier):
            raise RuntimeError("--attach-run 格式：工作流.yml=运行编号。")
        result = self.github.json("run", "view", identifier, "--json", "headSha,headBranch,event,displayTitle")
        runs = self.state.get("runs")
        entry = runs.get(workflow) if isinstance(runs, dict) else None
        if not isinstance(entry, dict) or not isinstance(result, dict) or any(result.get(key) != expected for key, expected in {
            "headSha": self.value("revision"), "headBranch": self.dispatch_branch(workflow), "event": "workflow_dispatch",
            "displayTitle": f"release-flow:{entry.get('operation')}",
        }.items()):
            raise RuntimeError("附加运行必须属于本次派发的操作号和提交。")
        remote = json.loads(command(["gh", "api", f"repos/{self.github.repository}/actions/runs/{identifier}"]))
        if not isinstance(remote, dict) or remote.get("path") != f".github/workflows/{workflow}":
            raise RuntimeError("附加运行的工作流不匹配。")
        if entry.get("id") not in (None, int(identifier)):
            raise RuntimeError("不能用另一个运行替换已锁定的运行。")
        entry["id"] = int(identifier)
        self.save()

    def recover_candidate(self, identifier: int) -> None:
        """Recover only the archived, already signed candidate; never sign again."""
        endpoint = f"repos/{self.github.repository}/actions/runs/{identifier}"
        run = json.loads(command(["gh", "api", endpoint]))
        if not isinstance(run, dict) or any(run.get(key) != expected for key, expected in {
            "head_sha": self.value("revision"), "head_branch": "main", "event": "workflow_dispatch",
            "status": "completed", "path": ".github/workflows/release-candidate.yml",
        }.items()) or any(not isinstance(run.get(key), dict) or run[key].get("full_name") != self.github.repository for key in ("repository", "head_repository")):
            raise RuntimeError("候选恢复来源不是本仓库已锁定的 main 工作流。")
        inventory = json.loads(command(["gh", "api", f"{endpoint}/artifacts?per_page=100"]))
        artifacts = inventory.get("artifacts") if isinstance(inventory, dict) else None
        if not isinstance(artifacts, list) or inventory.get("total_count") != len(artifacts):
            raise RuntimeError("无法核对完整候选归档。")
        name = f"release-candidate-{self.value('tag')}"
        matches = [item for item in artifacts if isinstance(item, dict) and item.get("name") == name]
        if len(matches) != 1 or matches[0].get("expired") is not False:
            raise Waiting("候选没有可用的完整签名归档。请检查原运行并使用文档中的原产物恢复流程；不会自动重建或重新签署。")
        if not self.state.get("downloaded"):
            self.candidate.mkdir(parents=True, exist_ok=True)
            self.github.call("run", "download", str(identifier), "--name", name, "--dir", str(self.candidate))
        self.verify_candidate()
        # Create is idempotent by tag. An uncertain response leaves the checkpoint
        # intact; the next invocation checks the exact existing draft first.
        listed = json.loads(command(["gh", "api", "--paginate", "--slurp", f"repos/{self.github.repository}/releases?per_page=100"]))
        if not isinstance(listed, list) or any(not isinstance(page, list) or any(not isinstance(row, dict) for row in page) for page in listed):
            raise RuntimeError("无法核对现有 Release 列表。")
        existing = [row for page in listed for row in page if row.get("tag_name") == self.value("tag")]
        if len(existing) > 1 or existing and existing[0].get("draft") is not True:
            raise RuntimeError("恢复上传只允许未公开的当前候选。")
        if not existing:
            self.github.call("release", "create", self.value("tag"), "--draft", "--target", self.value("revision"), "--title", f"Agent Room {self.value('tag')}", "--notes", "Recovered signed testing candidate. Publication requires the normal verification gates.")
        self.upload(sorted(path for path in self.candidate.iterdir() if path.is_file()))
        self.state["downloaded"] = True
        self.state["recoveredCandidateRun"] = identifier
        self.save()

    def download(self) -> None:
        self.candidate.mkdir(parents=True, exist_ok=True)
        if not self.state.get("downloaded"):
            self.github.call("release", "download", self.value("tag"), "--dir", str(self.candidate), "--clobber")
            self.state["downloaded"] = True
            self.save()

    def verify_candidate(self) -> None:
        metadata = release.load_object(self.candidate / "release-metadata.json", "metadata")
        expected = {key: self.state[key] for key in ("version", "tag", "revision", "sequence")}
        expected["channel"] = "testing"
        if any(metadata.get(key) != value for key, value in expected.items()):
            raise RuntimeError("候选元数据与本次锁定的发布不一致。")
        key = Path(self.value("trustedPublicKey"))
        if release.sha256_file(key) != self.value("trustedPublicKeySha256"):
            raise RuntimeError("独立信任公钥已变更。")
        signed = release.sha256_file(self.candidate / "release.signed.json")
        previous = self.state.get("signedManifestSha256")
        if previous is not None and previous != signed:
            raise RuntimeError("本次发布候选已被替换，不能沿用原验收结果。")
        inventory = release.load_object(self.candidate / "release-inventory.json", "inventory")
        if release.release_profile(inventory) != self.value("profile"):
            raise RuntimeError("候选发布范围与计划不一致。")
        release.verify(argparse.Namespace(
            root=self.candidate, inventory=self.candidate / "release-inventory.json", manifest=self.candidate / "release.json",
            evidence=self.candidate / "release-evidence.json", signed_manifest=self.candidate / "release.signed.json",
            public_key=key, release_tool=Path(self.value("releaseTool")), installed_version=self.value("installedVersion"),
            highest_sequence=self.state["highestSequence"], now_unix_seconds=int(time.time()),
            cosign=self.value("cosign"), docker="docker", expected_revision=self.value("revision"),
            certificate_identity_regexp=rf"^https://github.com/{re.escape(self.github.repository)}/\.github/workflows/release-candidate\.yml@refs/heads/main$",
            certificate_oidc_issuer="https://token.actions.githubusercontent.com",
        ))
        self.state["signedManifestSha256"] = signed
        release_native.verify(self.candidate, Path(self.value("releaseTool")), ROOT / "apps/desktop/src-tauri/tauri.release.conf.json")
        self.save()

    def deploy(self, phase: str) -> None:
        deployment = self.state.get("deployment")
        if not isinstance(deployment, dict):
            raise Waiting("需要部署配置或同一候选的兼容性报告；请按 docs/release-workflow.md 补齐，随后继续。")
        if not all(isinstance(deployment.get(key), str) for key in ("config", "state")):
            raise RuntimeError("部署配置路径无效。")
        command([sys.executable, str(ROOT / "tools/release_deploy.py"), phase,
                 "--candidate", str(self.candidate), "--config", deployment["config"], "--state-dir", deployment["state"],
                 "--revision", self.value("revision"), "--manifest-sha256", self.value("signedManifestSha256")], capture=False)

    def compatible_server(self) -> None:
        record = self.candidate / "release-promotion-compatible-server.json"
        if not record.exists():
            self.deploy("server")
        release_promotion.verify(record, "compatible-server", self.value("version"), self.value("revision"))
        release_promotion.verify_evidence(record, self.candidate, f"https://github.com/{self.github.repository}/releases/download/{self.value('tag')}/")
        promotion = release_promotion.load_record(record)
        # Upload only immutable, verified evidence. Existing identical assets are
        # reconciled before retry; never clobber an accepted release asset.
        files = [record]
        history = promotion.get("history")
        if not isinstance(history, list):
            raise RuntimeError("晋级历史无效。")
        for item in history:
            if isinstance(item, dict) and item.get("stage") in {"database-expanded", "compatible-server"}:
                files.append(self.candidate / str(item["evidenceUrl"]).rsplit("/", 1)[-1])
        self.upload(files)

    def upload(self, paths: list[Path]) -> None:
        document = self.github.json("release", "view", self.value("tag"), "--json", "assets")
        assets = document.get("assets") if isinstance(document, dict) else None
        if not isinstance(assets, list):
            raise RuntimeError("无法核对现有 Release 资产。")
        known = {row.get("name"): row for row in assets if isinstance(row, dict)}
        for path in paths:
            existing = known.get(path.name)
            if existing:
                # GitHub exposes asset SHA-256; older assets are compared by a
                # separate download rather than assuming an equal filename.
                digest = existing.get("digest")
                if digest == f"sha256:{release.sha256_file(path)}":
                    continue
                check = self.directory / "remote-assets"
                check.mkdir(exist_ok=True)
                self.github.call("release", "download", self.value("tag"), "--pattern", path.name, "--dir", str(check), "--clobber")
                if release.sha256_file(check / path.name) != release.sha256_file(path):
                    raise RuntimeError(f"远端已有不同内容的资产：{path.name}")
            else:
                self.github.call("release", "upload", self.value("tag"), str(path))

    def acceptance(self) -> None:
        try:
            release_acceptance.verify(self.candidate, self.value("version"), self.value("revision"))
        except (release.ReleaseFailure, OSError) as error:
            raise Waiting(f"等待真实易用性验收：{error} 报告目录：{self.candidate}") from error
        index = release.load_object(self.candidate / "release-usability-acceptance.json", "acceptance")
        reports = index["reports"]
        paths = [self.candidate / "release-usability-acceptance.json"]
        for reference in reports.values():
            path = self.candidate / reference["path"]
            paths.append(path)
            report = release.load_object(path, "usability report")
            paths.extend(self.candidate / item["path"] for item in report["evidence"])
        self.upload(list(dict.fromkeys(paths)))

    def verify_publication(self) -> None:
        published = self.github.json("release", "view", self.value("tag"), "--json", "isDraft,tagName,url")
        if not isinstance(published, dict) or published.get("isDraft") is not False or published.get("tagName") != self.value("tag"):
            raise Waiting("远端尚未确认公开发行。")
        directory = self.directory / "published"
        directory.mkdir(exist_ok=True)
        self.github.call("release", "download", self.value("tag"), "--pattern", "release.signed.json", "--dir", str(directory), "--clobber")
        channel = self.directory / "channel"
        channel.mkdir(exist_ok=True)
        self.github.call("release", "download", "channel-testing", "--pattern", "release.signed.json", "--dir", str(channel), "--clobber")
        for path in (directory / "release.signed.json", channel / "release.signed.json"):
            if release.sha256_file(path) != self.value("signedManifestSha256"):
                raise RuntimeError("公开版本或更新渠道没有指向本次候选。")

    def advance(self) -> None:
        if command(["git", "rev-parse", "HEAD"]).strip() != self.value("revision") or command(["git", "status", "--porcelain", "--untracked-files=no"]).strip():
            raise RuntimeError("发布必须使用锁定提交的干净工作树。")
        self.step("完整 CI", lambda: self.workflow("ci.yml", {"suite": "all"}, required=CI_JOBS))
        self.step("签名候选", lambda: self.workflow("release-candidate.yml", {"tag": self.value("tag"), "sequence": str(self.state["sequence"]), "channel": "testing", "profile": self.value("profile")}))
        self.step("下载候选", self.download)
        self.step("独立签名及产物核验", self.verify_candidate)
        self.step("服务端兼容部署", self.compatible_server)
        self.step("新设备、升级及真实接待验收", self.acceptance)
        self.step("公开发行", lambda: self.workflow(PUBLISH_WORKFLOW, {"tag": self.value("tag"), "installed_version": self.value("installedVersion"), "highest_sequence": str(self.state["highestSequence"])}))
        self.step("更新渠道核对", self.verify_publication)
        if self.value("profile") == "full":
            self.step("网页部署及运行核验", lambda: self.deploy("web"))
        self.stage("完成")
        print(f"发布完成：https://github.com/{self.github.repository}/releases/tag/{self.value('tag')}")

    def step(self, name: str, operation: Callable[[], None]) -> None:
        if name in self.completed:
            return
        self.stage(name)
        operation()
        self.completed.add(name)


def initialize(args: argparse.Namespace) -> dict[str, object]:
    for key in ("repository", "sequence", "installed_version", "highest_sequence", "trusted_public_key"):
        if getattr(args, key) is None:
            raise RuntimeError(f"首次运行缺少 --{key.replace('_', '-')}。")
    validate_repository(args.repository)
    revision = command(["git", "rev-parse", "HEAD"]).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise RuntimeError("Git 提交无效。")
    if args.sequence <= args.highest_sequence or args.highest_sequence < 0:
        raise RuntimeError("发布序号必须高于已信任序号。")
    version = workspace_version(ROOT / "Cargo.toml")
    state: dict[str, object] = {"schemaVersion": 1, "repository": args.repository, "revision": revision,
        "version": version, "tag": f"v{version}", "sequence": args.sequence, "profile": args.profile or "full",
        "installedVersion": args.installed_version, "highestSequence": args.highest_sequence,
        "trustedPublicKey": str(args.trusted_public_key.resolve(strict=True)),
        "trustedPublicKeySha256": release.sha256_file(args.trusted_public_key),
        "releaseTool": str((args.release_tool or ROOT / "target/release" / ("agent-room-release-tool.exe" if os.name == "nt" else "agent-room-release-tool")).resolve(strict=True)), "cosign": args.cosign or "cosign", "runs": {}}
    if args.deploy_config is not None or args.deploy_state is not None:
        if args.deploy_config is None or args.deploy_state is None:
            raise RuntimeError("部署配置与状态目录必须同时指定。")
        state["deployment"] = {"config": str(args.deploy_config.resolve(strict=True)), "state": str(args.deploy_state.resolve(strict=True))}
    return state


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, required=True, help="首次保存参数，此后只需传这个路径即可续跑")
    parser.add_argument("--repository")
    parser.add_argument("--sequence", type=int)
    parser.add_argument("--installed-version")
    parser.add_argument("--highest-sequence", type=int)
    parser.add_argument("--trusted-public-key", type=Path)
    parser.add_argument("--release-tool", type=Path)
    parser.add_argument("--cosign")
    parser.add_argument("--profile", choices=("client", "full"))
    parser.add_argument("--deploy-config", type=Path)
    parser.add_argument("--deploy-state", type=Path)
    parser.add_argument("--attach-run", action="append", default=[])
    parser.add_argument("--watch", action="store_true", help="等待远端运行；缺少人工验收或遇到失败仍停下")
    args = parser.parse_args()
    try:
        with exclusive(args.state.with_suffix(".lock")):
            if args.state.exists():
                state = dict(release.load_object(args.state, "发布检查点"))
                if state.get("schemaVersion") != 1:
                    raise RuntimeError("不支持的发布检查点版本。")
                for option, key in ((args.repository, "repository"), (args.sequence, "sequence"), (args.installed_version, "installedVersion"), (args.highest_sequence, "highestSequence")):
                    if option is not None and option != state.get(key):
                        raise RuntimeError(f"不能在续跑时改变 {key}。")
                for option, key in ((args.profile, "profile"), (args.cosign, "cosign"), (args.trusted_public_key, "trustedPublicKey"), (args.release_tool, "releaseTool")):
                    if option is not None and (str(option.resolve(strict=True)) if isinstance(option, Path) else option) != state.get(key):
                        raise RuntimeError(f"不能在续跑时改变 {key}。")
                for option, key in ((args.deploy_config, "config"), (args.deploy_state, "state")):
                    if option is not None and str(option.resolve(strict=True)) != state.get("deployment", {}).get(key):
                        raise RuntimeError("不能在续跑时改变部署目标。")
            else:
                state = initialize(args)
                atomic_json(args.state, state)
            repository = state.get("repository")
            if not isinstance(repository, str):
                raise RuntimeError("发布检查点缺少仓库。")
            flow = ReleaseFlow(args.state, state, GitHub(repository))
            for specification in args.attach_run:
                flow.attach_run(specification)
            while True:
                try:
                    flow.advance()
                    return 0
                except Waiting as error:
                    print(str(error), flush=True)
                    if not args.watch or not error.pollable:
                        return 20
                    time.sleep(15)
    except (RuntimeError, OSError, ValueError, release.ReleaseFailure, release_promotion.PromotionFailure) as error:
        parser.exit(1, f"发布已停止，检查点保留：{error}\n")
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
