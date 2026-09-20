#!/usr/bin/env python3
"""在一次性目录中验收 Agent Room macOS 磁盘映像。"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Final, Sequence
import uuid


SCHEMA_VERSION: Final = 1
PLATFORM: Final = "darwin-aarch64"
DESKTOP_EXECUTABLE: Final = "agent-room-desktop"
BRIDGE_EXECUTABLE: Final = "agent-room-bridge"
MCP_EXECUTABLE: Final = "agent-room-mcp"
CLI_EXECUTABLE: Final = "agent-room"
SEMVER_PATTERN: Final = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
BRIDGE_STABILITY_SECONDS: Final = 2.0
PROCESS_STABILITY_SECONDS: Final = 1.0
PROCESS_EXIT_TIMEOUT_SECONDS: Final = 30


class MacosBundleAcceptanceFailure(RuntimeError):
    """表示磁盘映像没有满足 macOS 发行门禁。"""


@dataclass(frozen=True, slots=True)
class InstalledLayout:
    """描述安装后必须存在的同版本运行时文件。"""

    app: Path
    desktop: Path
    bridge: Path
    mcp: Path
    cli: Path


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--applications", type=Path, default=Path("/Applications"))
    parser.add_argument("--launch-timeout-seconds", type=int, default=120)
    return parser.parse_args(argv)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run_checked(command: Sequence[str], label: str, *, timeout_seconds: int) -> str:
    completed = subprocess.run(
        command, capture_output=True, text=True, timeout=timeout_seconds, check=False
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()[-400:]
        raise MacosBundleAcceptanceFailure(f"{label}失败：{detail}")
    return completed.stdout


def locate_app(root: Path) -> Path:
    apps = sorted(path for path in root.glob("*.app") if path.is_dir())
    if len(apps) != 1:
        raise MacosBundleAcceptanceFailure(f"磁盘映像里的应用数量异常：{len(apps)}")
    return apps[0]


def locate_installed_layout(app: Path) -> InstalledLayout:
    executables = app / "Contents" / "MacOS"
    paths = {
        name: executables / name
        for name in (DESKTOP_EXECUTABLE, BRIDGE_EXECUTABLE, MCP_EXECUTABLE, CLI_EXECUTABLE)
    }
    missing = sorted(name for name, path in paths.items() if not os.access(path, os.X_OK))
    if missing:
        raise MacosBundleAcceptanceFailure(f"安装后缺少可执行文件：{', '.join(missing)}")
    return InstalledLayout(
        app=app,
        desktop=paths[DESKTOP_EXECUTABLE],
        bridge=paths[BRIDGE_EXECUTABLE],
        mcp=paths[MCP_EXECUTABLE],
        cli=paths[CLI_EXECUTABLE],
    )


def installed_desktop_version(desktop: Path, *, timeout_seconds: int = 30) -> str:
    completed = subprocess.run(
        (str(desktop), "--installer-version"),
        capture_output=True,
        text=True,
        timeout=timeout_seconds,
        check=False,
    )
    version = completed.stdout.strip()
    if completed.returncode != 0 or not SEMVER_PATTERN.fullmatch(version):
        detail = (completed.stderr or completed.stdout).strip()[-200:]
        raise MacosBundleAcceptanceFailure(f"无法读取已安装桌面端版本：{detail or '无有效输出'}")
    return version


def verify_cli_version(cli: Path, expected_version: str) -> None:
    completed = subprocess.run(
        (str(cli), "--version"), capture_output=True, text=True, timeout=30, check=False
    )
    if completed.returncode != 0 or completed.stdout.strip() != f"agent-room {expected_version}":
        raise MacosBundleAcceptanceFailure("已安装 CLI 无法启动或版本与桌面不一致。")


def process_ids(executable_name: str) -> frozenset[int]:
    completed = subprocess.run(
        ("pgrep", "-x", executable_name), capture_output=True, text=True, timeout=30, check=False
    )
    # pgrep 没有匹配进程时返回 1，这不是错误。
    if completed.returncode not in (0, 1):
        raise MacosBundleAcceptanceFailure("无法读取 macOS 进程列表。")
    return frozenset(int(line) for line in completed.stdout.split() if line.isdigit())


def acceptance_environment(temporary: Path) -> dict[str, str]:
    environment = {
        name: value for name, value in os.environ.items() if not name.startswith("AGENT_ROOM_")
    }
    environment["AGENT_ROOM_BRIDGE_DATA_DIR"] = str(temporary / "bridge-data")
    environment["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"] = (
        f"dev.agent-room.acceptance.{uuid.uuid4().hex}"
    )
    return environment


def wait_for_bridge(
    previous: frozenset[int], desktop: subprocess.Popen[bytes], timeout_seconds: int
) -> int:
    deadline = time.monotonic() + timeout_seconds
    bridge_pid: int | None = None
    observed_at: float | None = None
    while time.monotonic() < deadline:
        if desktop.poll() is not None:
            raise MacosBundleAcceptanceFailure(
                f"桌面端在 Bridge 启动前退出（退出码 {desktop.returncode}）。"
            )
        running = process_ids(BRIDGE_EXECUTABLE) - previous
        if bridge_pid is None and running:
            bridge_pid = min(running)
            observed_at = time.monotonic()
        if (
            bridge_pid is not None
            and bridge_pid in running
            and observed_at is not None
            and time.monotonic() - observed_at >= BRIDGE_STABILITY_SECONDS
        ):
            return bridge_pid
        if bridge_pid is not None and bridge_pid not in running:
            bridge_pid = None
            observed_at = None
        time.sleep(1)
    raise MacosBundleAcceptanceFailure("桌面端未在时限内启动受管 Bridge。")


def wait_for_process_stability(
    process: subprocess.Popen[bytes],
    label: str,
    *,
    stability_seconds: float = PROCESS_STABILITY_SECONDS,
) -> None:
    deadline = time.monotonic() + stability_seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise MacosBundleAcceptanceFailure(f"{label}未保持运行（退出码 {process.returncode}）。")
        time.sleep(0.1)


def wait_for_process_exit(
    process: subprocess.Popen[bytes],
    label: str,
    *,
    timeout_seconds: int = PROCESS_EXIT_TIMEOUT_SECONDS,
) -> None:
    try:
        process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired as error:
        raise MacosBundleAcceptanceFailure(f"{label}没有按验收要求退出。") from error


def wait_for_image_exit(
    executable_name: str,
    process_id: int,
    label: str,
    *,
    timeout_seconds: int = PROCESS_EXIT_TIMEOUT_SECONDS,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if process_id not in process_ids(executable_name):
            return
        time.sleep(0.5)
    raise MacosBundleAcceptanceFailure(f"{label}没有随桌面端退出。")


def stop_managed_bridge(process_id: int) -> None:
    """按平台的方式请求受管 Bridge 退出；它不是本脚本的子进程。"""

    try:
        os.kill(process_id, signal.SIGTERM)
    except ProcessLookupError as error:
        raise MacosBundleAcceptanceFailure("受管 Bridge 在验收停止前就已经不在了。") from error


def terminate(process: subprocess.Popen[bytes] | None) -> None:
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
    if process.stdin is not None:
        process.stdin.close()


def install_from_image(image: Path, mountpoint: Path, applications: Path) -> Path:
    """挂载只读映像、按 macOS 的方式复制整包，再卸载，保持与用户拖拽安装一致。"""

    run_checked(
        ("hdiutil", "attach", str(image), "-nobrowse", "-readonly", "-mountpoint", str(mountpoint)),
        "挂载磁盘映像",
        timeout_seconds=300,
    )
    try:
        source = locate_app(mountpoint)
        destination = applications / source.name
        if destination.exists():
            raise MacosBundleAcceptanceFailure(f"验收目标已存在，拒绝覆盖：{destination}")
        applications.mkdir(parents=True, exist_ok=True)
        run_checked(("ditto", str(source), str(destination)), "复制应用", timeout_seconds=600)
        return destination
    finally:
        run_checked(("hdiutil", "detach", str(mountpoint)), "卸载磁盘映像", timeout_seconds=300)


def replace_install(image: Path, mountpoint: Path, destination: Path) -> None:
    """覆盖安装：与用户再次拖拽同名应用一致，必须仍然可用。"""

    run_checked(
        ("hdiutil", "attach", str(image), "-nobrowse", "-readonly", "-mountpoint", str(mountpoint)),
        "再次挂载磁盘映像",
        timeout_seconds=300,
    )
    try:
        source = locate_app(mountpoint)
        run_checked(
            ("ditto", str(source), str(destination)), "覆盖安装应用", timeout_seconds=600
        )
    finally:
        run_checked(("hdiutil", "detach", str(mountpoint)), "卸载磁盘映像", timeout_seconds=300)


def write_new_report(path: Path, document: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        stream = path.open("x", encoding="utf-8")
    except FileExistsError as error:
        raise MacosBundleAcceptanceFailure(f"拒绝覆盖已有验收报告：{path}") from error
    with stream:
        json.dump(document, stream, ensure_ascii=False, indent=2, sort_keys=True)
        stream.write("\n")


def accept(
    image: Path, expected_version: str, report: Path, applications: Path, launch_timeout_seconds: int
) -> None:
    if not SEMVER_PATTERN.fullmatch(expected_version):
        raise MacosBundleAcceptanceFailure("预期版本不是有效 SemVer。")
    if sys.platform != "darwin":
        raise MacosBundleAcceptanceFailure("macOS 磁盘映像验收只能在 macOS 上执行。")
    image = image.resolve(strict=True)

    with tempfile.TemporaryDirectory() as temporary_name:
        temporary = Path(temporary_name)
        mountpoint = temporary / "image"
        environment = acceptance_environment(temporary)
        desktop_process: subprocess.Popen[bytes] | None = None
        mcp_process: subprocess.Popen[bytes] | None = None
        installed: Path | None = None
        bridge_pid: int | None = None
        upgraded_bridge_pid: int | None = None
        try:
            installed = install_from_image(image, mountpoint, applications)
            layout = locate_installed_layout(installed)
            actual_version = installed_desktop_version(layout.desktop)
            if actual_version != expected_version:
                raise MacosBundleAcceptanceFailure(
                    f"安装后的桌面端版本不符：{actual_version} != {expected_version}"
                )
            verify_cli_version(layout.cli, expected_version)

            previous_bridge_ids = process_ids(BRIDGE_EXECUTABLE)
            desktop_process = subprocess.Popen(
                (str(layout.desktop), "--installer-acceptance"),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                env=environment,
            )
            bridge_pid = wait_for_bridge(previous_bridge_ids, desktop_process, launch_timeout_seconds)
            mcp_process = subprocess.Popen(
                (str(layout.mcp),),
                # MCP 走 stdio，stdin 一到 EOF 就正常退出；验收要看它保持运行，必须留着管道。
                stdin=subprocess.PIPE,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                env=environment,
            )
            wait_for_process_stability(mcp_process, "MCP")
            terminate(mcp_process)
            mcp_process = None
            # macOS 没有安装器钩子去停正在运行的应用（Windows 靠的是原地升级），
            # 所以这里验收另一条同样要成立的约束：受管 Bridge 一停，桌面端必须跟着
            # 退出，不能把运行时留在后台。桌面端随退出码 1 结束，因为 Bridge 是被信号带走的。
            stop_managed_bridge(bridge_pid)
            wait_for_process_exit(desktop_process, "桌面端")
            wait_for_image_exit(BRIDGE_EXECUTABLE, bridge_pid, "受管 Bridge")
            desktop_process = None

            replace_install(image, mountpoint, installed)
            layout = locate_installed_layout(installed)
            upgraded_version = installed_desktop_version(layout.desktop)
            if upgraded_version != expected_version:
                raise MacosBundleAcceptanceFailure(
                    f"覆盖安装后的版本不符：{upgraded_version} != {expected_version}"
                )
            verify_cli_version(layout.cli, expected_version)
            previous_bridge_ids = process_ids(BRIDGE_EXECUTABLE)
            desktop_process = subprocess.Popen(
                (str(layout.desktop), "--installer-acceptance"),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                env=environment,
            )
            upgraded_bridge_pid = wait_for_bridge(
                previous_bridge_ids, desktop_process, launch_timeout_seconds
            )
            stop_managed_bridge(upgraded_bridge_pid)
            wait_for_process_exit(desktop_process, "覆盖安装后的桌面端")
            wait_for_image_exit(BRIDGE_EXECUTABLE, upgraded_bridge_pid, "覆盖安装后的受管 Bridge")
            desktop_process = None
        finally:
            terminate(mcp_process)
            terminate(desktop_process)
            if installed is not None and installed.exists():
                shutil.rmtree(installed, ignore_errors=True)
            if mountpoint.exists():
                subprocess.run(
                    ("hdiutil", "detach", str(mountpoint)),
                    capture_output=True,
                    timeout=300,
                    check=False,
                )

        if installed is None or installed.exists():
            raise MacosBundleAcceptanceFailure("卸载后应用仍然留在磁盘上。")
        if bridge_pid is None or upgraded_bridge_pid is None:
            raise MacosBundleAcceptanceFailure("没有记录到受管 Bridge 进程。")

        write_new_report(
            report,
            {
                "schemaVersion": SCHEMA_VERSION,
                "result": "passed",
                "platform": PLATFORM,
                "version": expected_version,
                "installer": {
                    "filename": image.name,
                    "sha256": sha256_file(image),
                    "byteLength": image.stat().st_size,
                },
                "checks": {
                    "imageMounted": True,
                    "appInstalled": True,
                    "desktopPresent": True,
                    "bridgePresent": True,
                    "mcpPresent": True,
                    "cliPresent": True,
                    "desktopVersion": True,
                    "cliVersion": True,
                    "desktopLaunch": True,
                    "managedBridgeLaunch": True,
                    "mcpLaunch": True,
                    "desktopExitedWithBridge": True,
                    "replaceInstall": True,
                    "postReplaceDesktopLaunch": True,
                    "postReplaceBridgeLaunch": True,
                    "postReplaceDesktopExitedWithBridge": True,
                    "applicationRemoved": True,
                },
            },
        )


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(argv)
    try:
        accept(
            arguments.image,
            arguments.expected_version,
            arguments.report,
            arguments.applications,
            arguments.launch_timeout_seconds,
        )
    except (MacosBundleAcceptanceFailure, OSError, subprocess.TimeoutExpired) as error:
        print(f"macOS 磁盘映像验收失败：{error}", file=sys.stderr)
        return 1
    print(f"macOS 磁盘映像验收通过：{arguments.report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
