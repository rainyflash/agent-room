#!/usr/bin/env python3
"""生成封闭测试制品、执行 M2 验收矩阵并验证放行门。"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import tomllib
from typing import Final, Iterable, Mapping, Sequence
import zipfile

if __package__:
    from .sensitive_output import scan_files
else:
    from sensitive_output import scan_files


ROOT: Final = Path(__file__).resolve().parent.parent
ARTIFACT_ROOT: Final = ROOT / "artifacts" / "closed-test"
PACKAGE_ROOT: Final = ARTIFACT_ROOT / "packages"
REPORT_PATH: Final = ARTIFACT_ROOT / "m2-acceptance-report.json"
BLOCKER_LEDGER: Final = ROOT / "release" / "closed-test" / "blockers.json"
FIXED_ZIP_TIME: Final = (2026, 1, 1, 0, 0, 0)
CANARY_SECRET: Final = "m2-closed-test-canary-secret"
DESKTOP_SUFFIXES: Final = frozenset(
    {".app", ".dmg", ".exe", ".msi", ".sig", ".tar.gz", ".zip"}
)
DEPLOYMENT_PREFIXES: Final = (
    "apps/bridge/",
    "apps/control-plane/",
    "apps/web/",
    "crates/",
    "infra/",
    "packages/",
    "tools/",
)
DEPLOYMENT_FILES: Final = frozenset(
    {
        ".env.example",
        ".node-version",
        ".python-version",
        "Cargo.lock",
        "Cargo.toml",
        "LICENSE",
        "README.md",
        "deny.toml",
        "justfile",
        "package.json",
        "pnpm-lock.yaml",
        "pnpm-workspace.yaml",
        "rust-toolchain.toml",
        "tsconfig.json",
    }
)


@dataclass(frozen=True)
class Scenario:
    identifier: str
    label: str
    requirements: tuple[int, ...]
    command: tuple[str, ...]


@dataclass(frozen=True)
class ScenarioResult:
    identifier: str
    label: str
    requirements: tuple[int, ...]
    passed: bool
    duration_seconds: float
    log: str


@dataclass(frozen=True)
class Artifact:
    kind: str
    platform: str
    path: str
    sha256: str
    size: int


SCENARIOS: Final = (
    Scenario(
        "quality",
        "全工作区格式、类型、单元、协议与构建",
        tuple(range(1, 16)),
        ("just", "check"),
    ),
    Scenario(
        "browser-journeys",
        "中英文、无障碍、私人房间、治理与消息浏览器旅程",
        (3, 4, 5, 6, 7, 9, 10, 12, 14, 15),
        ("corepack", "pnpm@10.28.0", "--filter", "@agent-room/web", "test:browser"),
    ),
    Scenario(
        "multi-user-agent",
        "真实服务多用户、Bridge、Codex 插件与一次性交接",
        (1, 2, 3, 5, 6, 7, 8, 10, 11, 15),
        (sys.executable, "tools/vertical.py", "bootstrap"),
    ),
    Scenario(
        "multi-device-e2ee",
        "真实 Synapse 三设备交叉签名、SAS 与恢复",
        (4, 5, 11, 12, 15),
        (sys.executable, "tools/vertical.py", "security"),
    ),
    Scenario(
        "recovery",
        "断网、休眠、重启、未知提交与磁盘故障恢复",
        (6, 11, 15),
        (sys.executable, "tools/reliability.py"),
    ),
    Scenario(
        "privacy-security",
        "输入、提示注入、内容、限流与敏感输出门禁",
        (7, 10, 11, 12, 15),
        (sys.executable, "tools/security.py"),
    ),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    package_parser = subcommands.add_parser("package", help="生成当前平台封闭测试制品")
    package_parser.add_argument("--platform-tag", default=current_platform_tag())
    package_parser.add_argument(
        "--desktop-bundle-root",
        type=Path,
        default=ROOT / "target" / "release" / "bundle",
    )
    package_parser.add_argument("--skip-desktop", action="store_true")
    package_parser.add_argument("--skip-build", action="store_true")

    subcommands.add_parser("matrix", help="执行完整 M2 封闭测试矩阵")
    verify_parser = subcommands.add_parser("verify", help="验证报告、阻断项和制品清单")
    verify_parser.add_argument(
        "--required-platform",
        action="append",
        default=[],
        help="要求存在桌面制品的平台标签，可重复传入",
    )
    return parser.parse_args()


def current_platform_tag() -> str:
    systems = {"Windows": "windows", "Darwin": "macos", "Linux": "linux"}
    architectures = {
        "AMD64": "x64",
        "x86_64": "x64",
        "arm64": "arm64",
        "aarch64": "arm64",
    }
    system = systems.get(platform.system())
    architecture = architectures.get(platform.machine())
    if system is None or architecture is None:
        raise RuntimeError("无法推导封闭测试平台标签，请显式传入 --platform-tag。")
    return f"{system}-{architecture}"


def workspace_version() -> str:
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    workspace = manifest.get("workspace")
    package = workspace.get("package") if isinstance(workspace, dict) else None
    version = package.get("version") if isinstance(package, dict) else None
    if not isinstance(version, str) or not version:
        raise RuntimeError("Cargo workspace 缺少有效版本。")
    return version


def require_clean_tree() -> str:
    completed = run_capture(("git", "status", "--porcelain"))
    if completed.stdout.strip():
        raise RuntimeError("封闭测试制品只能从干净工作树生成。")
    revision = run_capture(("git", "rev-parse", "HEAD")).stdout.strip()
    if len(revision) != 40:
        raise RuntimeError("无法读取完整 Git 提交。")
    return revision


def package(platform_tag: str, desktop_bundle_root: Path, skip_desktop: bool, skip_build: bool) -> None:
    revision = require_clean_tree()
    version = workspace_version()
    platform_root = safe_package_directory(platform_tag)
    if platform_root.exists():
        shutil.rmtree(platform_root)
    platform_root.mkdir(parents=True)

    if not skip_build:
        run_checked(("corepack", "pnpm@10.28.0", "build"))
    web_dist = ROOT / "apps" / "web" / "dist"
    if not web_dist.is_dir():
        raise RuntimeError("Web 生产目录不存在。")

    artifacts: list[Artifact] = []
    web_archive = platform_root / f"agent-room-web-v{version}.zip"
    write_reproducible_zip(web_archive, files_under(web_dist), web_dist.parent)
    artifacts.append(describe_artifact("web", "all", web_archive))

    deployment_archive = platform_root / f"agent-room-single-node-v{version}.zip"
    deployment_files = tracked_deployment_files()
    write_reproducible_zip(deployment_archive, deployment_files, ROOT)
    artifacts.append(describe_artifact("single-node", "linux", deployment_archive))

    run_checked(
        (
            sys.executable,
            "tools/plugin.py",
            "stage",
            "--platform-tag",
            platform_tag,
        )
    )
    plugin_archive = (
        ROOT
        / "artifacts"
        / "codex-plugin"
        / f"agent-room-plugin-v{version}-{platform_tag}.zip"
    )
    if not plugin_archive.is_file():
        raise RuntimeError("Codex 插件打包后未产生归档。")
    copied_plugin = platform_root / plugin_archive.name
    shutil.copy2(plugin_archive, copied_plugin)
    artifacts.append(describe_artifact("codex-plugin", platform_tag, copied_plugin))

    if not skip_desktop:
        desktop_root = desktop_bundle_root.resolve()
        desktop_files = discover_desktop_artifacts(desktop_root)
        desktop_archive = platform_root / f"agent-room-desktop-v{version}-{platform_tag}.zip"
        write_reproducible_zip(desktop_archive, desktop_files, desktop_root)
        artifacts.append(describe_artifact("desktop", platform_tag, desktop_archive))

    manifest = {
        "schemaVersion": 1,
        "version": version,
        "revision": revision,
        "platform": platform_tag,
        "generatedAt": datetime.now(UTC).isoformat(),
        "artifacts": [asdict(artifact) for artifact in artifacts],
    }
    manifest_path = platform_root / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"封闭测试制品清单：{manifest_path}")


def safe_package_directory(platform_tag: str) -> Path:
    if not platform_tag or any(character not in "abcdefghijklmnopqrstuvwxyz0123456789-" for character in platform_tag):
        raise RuntimeError("平台标签只能包含小写字母、数字和连字符。")
    target = (PACKAGE_ROOT / platform_tag).resolve()
    package_root = PACKAGE_ROOT.resolve()
    if target == package_root or package_root not in target.parents:
        raise RuntimeError("拒绝写入封闭测试制品目录之外。")
    return target


def files_under(root: Path) -> tuple[Path, ...]:
    return tuple(path for path in sorted(root.rglob("*")) if path.is_file())


def tracked_deployment_files() -> tuple[Path, ...]:
    completed = subprocess.run(
        [resolve_executable("git"), "ls-files", "-z"],
        cwd=ROOT,
        capture_output=True,
        check=True,
    )
    files: list[Path] = []
    for raw_path in completed.stdout.split(b"\0"):
        if not raw_path:
            continue
        relative = raw_path.decode("utf-8")
        if relative in DEPLOYMENT_FILES or relative.startswith(DEPLOYMENT_PREFIXES):
            source = (ROOT / relative).resolve()
            if source.is_file():
                files.append(source)
    if not files:
        raise RuntimeError("单机部署包没有可归档文件。")
    return tuple(sorted(files))


def discover_desktop_artifacts(root: Path) -> tuple[Path, ...]:
    if not root.is_dir():
        raise RuntimeError(f"桌面打包目录不存在：{root}")
    files = tuple(
        path
        for path in sorted(root.rglob("*"))
        if path.is_file() and desktop_suffix(path) in DESKTOP_SUFFIXES
    )
    if not files:
        raise RuntimeError(f"桌面打包目录没有安装器或应用包：{root}")
    return files


def desktop_suffix(path: Path) -> str:
    lower = path.name.lower()
    if lower.endswith(".tar.gz"):
        return ".tar.gz"
    return path.suffix.lower()


def write_reproducible_zip(archive: Path, files: Iterable[Path], root: Path) -> None:
    archive.parent.mkdir(parents=True, exist_ok=True)
    if archive.exists():
        archive.unlink()
    root = root.resolve()
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for source in sorted(path.resolve() for path in files):
            if root != source and root not in source.parents:
                raise RuntimeError(f"拒绝归档根目录之外的文件：{source}")
            relative = source.relative_to(root).as_posix()
            info = zipfile.ZipInfo(relative, FIXED_ZIP_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            bundle.writestr(info, source.read_bytes())


def describe_artifact(kind: str, platform_tag: str, path: Path) -> Artifact:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return Artifact(
        kind=kind,
        platform=platform_tag,
        path=path.relative_to(ROOT).as_posix(),
        sha256=digest.hexdigest(),
        size=path.stat().st_size,
    )


def run_matrix() -> None:
    revision = require_clean_tree()
    log_root = ARTIFACT_ROOT / "logs"
    if log_root.exists():
        shutil.rmtree(log_root)
    log_root.mkdir(parents=True)

    environment = os.environ.copy()
    environment.pop("FORCE_COLOR", None)
    environment["NO_COLOR"] = "1"
    environment["AGENT_ROOM_CLOSED_TEST_CANARY"] = CANARY_SECRET
    results: list[ScenarioResult] = []
    for scenario in SCENARIOS:
        print(f"[M2] {scenario.label} ...", flush=True)
        started = time.monotonic()
        completed = subprocess.run(
            resolved_command(scenario.command),
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
        log_path = log_root / f"{scenario.identifier}.log"
        log_path.write_text(completed.stdout + completed.stderr, encoding="utf-8")
        result = ScenarioResult(
            identifier=scenario.identifier,
            label=scenario.label,
            requirements=scenario.requirements,
            passed=completed.returncode == 0,
            duration_seconds=round(time.monotonic() - started, 3),
            log=log_path.relative_to(ROOT).as_posix(),
        )
        results.append(result)
        print("  通过" if result.passed else f"  失败：{result.log}", flush=True)

    violations = scan_files(
        (("log", ROOT / result.log) for result in results),
        known_secrets=(CANARY_SECRET,),
    )
    blockers = read_open_blockers()
    passed = all(result.passed for result in results) and not violations and not blockers
    report = {
        "schemaVersion": 1,
        "milestone": "M2",
        "revision": revision,
        "generatedAt": datetime.now(UTC).isoformat(),
        "passed": passed,
        "scenarioPassRate": {
            "passed": sum(result.passed for result in results),
            "total": len(results),
        },
        "openBlockers": blockers,
        "sensitiveOutputViolations": [asdict(item) for item in violations],
        "scenarios": [asdict(result) for result in results],
    }
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_PATH.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"M2 验收报告：{REPORT_PATH}")
    if not passed:
        raise RuntimeError("M2 验收没有通过；禁止生成放行结论。")


def read_open_blockers() -> list[Mapping[str, object]]:
    payload = json.loads(BLOCKER_LEDGER.read_text(encoding="utf-8"))
    if not isinstance(payload, dict) or payload.get("schemaVersion") != 1:
        raise RuntimeError("封闭测试阻断项台账格式无效。")
    issues = payload.get("issues")
    if not isinstance(issues, list):
        raise RuntimeError("封闭测试阻断项台账缺少 issues 数组。")
    blockers: list[Mapping[str, object]] = []
    for item in issues:
        if not isinstance(item, dict):
            raise RuntimeError("封闭测试阻断项必须是对象。")
        if item.get("severity") == "blocking" and item.get("status") != "closed":
            blockers.append(item)
    return blockers


def verify(required_platforms: Sequence[str]) -> None:
    report = read_json_object(REPORT_PATH)
    if report.get("passed") is not True:
        raise RuntimeError("M2 验收报告未通过。")
    if report.get("revision") != require_clean_tree():
        raise RuntimeError("M2 验收报告不是基于当前提交。")
    if read_open_blockers():
        raise RuntimeError("仍有未关闭的 M2 阻断项。")

    manifests = tuple(sorted(PACKAGE_ROOT.glob("*/manifest.json")))
    if not manifests:
        raise RuntimeError("没有封闭测试制品清单。")
    desktop_platforms: set[str] = set()
    kinds: set[str] = set()
    for manifest_path in manifests:
        manifest = read_json_object(manifest_path)
        if manifest.get("revision") != report.get("revision"):
            raise RuntimeError(f"制品与验收提交不一致：{manifest_path}")
        artifacts = manifest.get("artifacts")
        if not isinstance(artifacts, list):
            raise RuntimeError(f"制品清单缺少 artifacts：{manifest_path}")
        for value in artifacts:
            if not isinstance(value, dict):
                raise RuntimeError(f"制品记录格式无效：{manifest_path}")
            verify_artifact(value)
            kind = value.get("kind")
            artifact_platform = value.get("platform")
            if isinstance(kind, str):
                kinds.add(kind)
                if kind == "desktop" and isinstance(artifact_platform, str):
                    desktop_platforms.add(artifact_platform)
    required_kinds = {"codex-plugin", "desktop", "single-node", "web"}
    if missing := required_kinds - kinds:
        raise RuntimeError(f"封闭测试制品种类不完整：{sorted(missing)}")
    if missing_platforms := set(required_platforms) - desktop_platforms:
        raise RuntimeError(f"缺少桌面平台制品：{sorted(missing_platforms)}")
    print("M2 验收报告、阻断项和制品摘要全部一致。")


def read_json_object(path: Path) -> dict[str, object]:
    if not path.is_file():
        raise RuntimeError(f"缺少文件：{path}")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"JSON 根节点必须是对象：{path}")
    return value


def verify_artifact(value: Mapping[str, object]) -> None:
    relative = value.get("path")
    expected_digest = value.get("sha256")
    expected_size = value.get("size")
    if not isinstance(relative, str) or not isinstance(expected_digest, str) or not isinstance(expected_size, int):
        raise RuntimeError("制品记录缺少路径、摘要或大小。")
    path = (ROOT / relative).resolve()
    artifact_root = ARTIFACT_ROOT.resolve()
    if artifact_root not in path.parents or not path.is_file():
        raise RuntimeError(f"制品路径越界或不存在：{relative}")
    actual = describe_artifact("verify", "verify", path)
    if actual.sha256 != expected_digest or actual.size != expected_size:
        raise RuntimeError(f"制品摘要或大小不匹配：{relative}")


def resolved_command(command: Sequence[str]) -> tuple[str, ...]:
    if not command:
        raise RuntimeError("验收命令不能为空。")
    return (resolve_executable(command[0]), *command[1:])


def resolve_executable(name: str) -> str:
    if Path(name).is_absolute():
        return name
    candidates = (f"{name}.cmd", f"{name}.exe", name) if os.name == "nt" else (name,)
    for candidate in candidates:
        resolved = shutil.which(candidate)
        if resolved is not None:
            return resolved
    raise RuntimeError(f"缺少可执行依赖：{name}")


def run_checked(command: Sequence[str]) -> None:
    completed = subprocess.run(
        resolved_command(command),
        cwd=ROOT,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"命令失败，退出码 {completed.returncode}。")


def run_capture(command: Sequence[str]) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        resolved_command(command),
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"命令失败，退出码 {completed.returncode}。")
    return completed


def main() -> int:
    args = parse_args()
    if args.command == "package":
        package(
            args.platform_tag,
            args.desktop_bundle_root,
            args.skip_desktop,
            args.skip_build,
        )
    elif args.command == "matrix":
        run_matrix()
    else:
        verify(args.required_platform)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError, ValueError) as error:
        print(f"封闭测试失败：{error}", file=sys.stderr)
        raise SystemExit(1) from None
