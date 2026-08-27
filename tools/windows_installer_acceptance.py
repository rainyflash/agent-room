#!/usr/bin/env python3
"""在一次性目录中验收 Agent Room Windows NSIS 安装器。"""

from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from typing import Final, Sequence


SCHEMA_VERSION: Final = 1
DESKTOP_EXECUTABLE: Final = "agent-room-desktop.exe"
BRIDGE_EXECUTABLE: Final = "agent-room-bridge.exe"
MCP_EXECUTABLE: Final = "agent-room-mcp.exe"
SEMVER_PATTERN: Final = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)


class WindowsInstallerAcceptanceFailure(RuntimeError):
    """表示安装器没有满足干净 Windows 发行门禁。"""


@dataclass(frozen=True, slots=True)
class InstalledLayout:
    """描述 NSIS 安装后必须存在的同版本运行时文件。"""

    root: Path
    desktop: Path
    bridge: Path
    mcp: Path
    uninstaller: Path


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installer", type=Path, required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--launch-timeout-seconds", type=int, default=20)
    return parser.parse_args(argv)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def unique_file(root: Path, filename: str) -> Path:
    matches = tuple(path for path in root.rglob("*") if path.is_file() and path.name.lower() == filename.lower())
    if len(matches) != 1:
        raise WindowsInstallerAcceptanceFailure(
            f"安装目录中的 {filename} 数量异常：{len(matches)}。"
        )
    return matches[0]


def locate_installed_layout(root: Path) -> InstalledLayout:
    desktop = unique_file(root, DESKTOP_EXECUTABLE)
    bridge = unique_file(root, BRIDGE_EXECUTABLE)
    mcp = unique_file(root, MCP_EXECUTABLE)
    uninstallers = tuple(
        path
        for path in root.rglob("*.exe")
        if path.is_file() and "uninstall" in path.name.lower()
    )
    if len(uninstallers) != 1:
        raise WindowsInstallerAcceptanceFailure(
            f"安装目录中的卸载器数量异常：{len(uninstallers)}。"
        )
    executable_parent = desktop.parent.resolve()
    for path in (bridge, mcp, uninstallers[0]):
        if path.parent.resolve() != executable_parent:
            raise WindowsInstallerAcceptanceFailure("桌面端、Bridge、MCP 与卸载器必须位于同一目录。")
    return InstalledLayout(executable_parent, desktop, bridge, mcp, uninstallers[0])


def run_checked(command: Sequence[str], label: str, *, timeout_seconds: int) -> None:
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout_seconds,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        raise WindowsInstallerAcceptanceFailure(
            f"{label}失败（退出码 {completed.returncode}）：{detail}"
        )


def process_ids(image_name: str) -> frozenset[int]:
    completed = subprocess.run(
        ("tasklist", "/FI", f"IMAGENAME eq {image_name}", "/FO", "CSV", "/NH"),
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
    )
    if completed.returncode != 0:
        raise WindowsInstallerAcceptanceFailure("无法读取 Windows 进程列表。")
    identifiers: set[int] = set()
    for row in csv.reader(completed.stdout.splitlines()):
        if len(row) >= 2 and row[0].lower() == image_name.lower() and row[1].isdigit():
            identifiers.add(int(row[1]))
    return frozenset(identifiers)


def wait_for_bridge(previous: frozenset[int], desktop: subprocess.Popen[bytes], timeout_seconds: int) -> int:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if desktop.poll() is not None:
            raise WindowsInstallerAcceptanceFailure(
                f"桌面端在 Bridge 启动前退出（退出码 {desktop.returncode}）。"
            )
        started = process_ids(BRIDGE_EXECUTABLE) - previous
        if started:
            return min(started)
        time.sleep(1)
    raise WindowsInstallerAcceptanceFailure("桌面端未在时限内启动受管 Bridge。")


def terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    subprocess.run(
        ("taskkill", "/PID", str(process.pid), "/T", "/F"),
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=30,
    )
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def wait_for_install_files_removed(root: Path, *, timeout_seconds: int = 30) -> None:
    deadline = time.monotonic() + timeout_seconds
    remaining: tuple[Path, ...] = ()
    while time.monotonic() < deadline:
        remaining = tuple(path for path in root.rglob("*") if path.is_file())
        if not remaining:
            return
        time.sleep(1)
    names = ", ".join(sorted(path.name for path in remaining))
    raise WindowsInstallerAcceptanceFailure(f"静默卸载后仍残留安装文件：{names}")


def write_new_report(path: Path, document: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(document, target, ensure_ascii=False, indent=2)
            target.write("\n")
    except FileExistsError as error:
        raise WindowsInstallerAcceptanceFailure(f"拒绝覆盖已有验收报告：{path}") from error


def accept(installer: Path, expected_version: str, report: Path, launch_timeout_seconds: int) -> None:
    if os.name != "nt":
        raise WindowsInstallerAcceptanceFailure("Windows 安装器验收只能在 Windows 上运行。")
    if not SEMVER_PATTERN.fullmatch(expected_version):
        raise WindowsInstallerAcceptanceFailure("expected-version 不是受支持的 SemVer。")
    installer = installer.resolve(strict=True)
    if not installer.is_file() or installer.suffix.lower() != ".exe":
        raise WindowsInstallerAcceptanceFailure("installer 必须是存在的 EXE 文件。")
    if launch_timeout_seconds < 5 or launch_timeout_seconds > 120:
        raise WindowsInstallerAcceptanceFailure("launch-timeout-seconds 必须在 5 到 120 之间。")

    with tempfile.TemporaryDirectory(prefix="agent-room-installer-acceptance-") as temporary:
        install_root = Path(temporary) / "installed"
        layout: InstalledLayout | None = None
        desktop: subprocess.Popen[bytes] | None = None
        bridge_pid: int | None = None
        try:
            run_checked(
                (str(installer), "/S", "/NS", f"/D={install_root}"),
                "静默安装",
                timeout_seconds=300,
            )
            layout = locate_installed_layout(install_root)
            previous_bridge_ids = process_ids(BRIDGE_EXECUTABLE)
            desktop = subprocess.Popen(
                (str(layout.desktop), "--installer-acceptance"),
                cwd=layout.root,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            bridge_pid = wait_for_bridge(previous_bridge_ids, desktop, launch_timeout_seconds)
        finally:
            if desktop is not None:
                terminate_process_tree(desktop)
            if layout is not None and layout.uninstaller.is_file():
                run_checked((str(layout.uninstaller), "/S"), "静默卸载", timeout_seconds=300)

        wait_for_install_files_removed(install_root)
        if bridge_pid is None:
            raise WindowsInstallerAcceptanceFailure("没有记录到受管 Bridge 进程。")

        write_new_report(
            report,
            {
                "schemaVersion": SCHEMA_VERSION,
                "result": "passed",
                "platform": "windows-x86_64",
                "version": expected_version,
                "installer": {
                    "filename": installer.name,
                    "sha256": sha256_file(installer),
                    "byteLength": installer.stat().st_size,
                },
                "checks": {
                    "silentInstall": True,
                    "desktopPresent": True,
                    "bridgePresent": True,
                    "mcpPresent": True,
                    "desktopLaunch": True,
                    "managedBridgeLaunch": True,
                    "silentUninstall": True,
                    "installFilesRemoved": True,
                },
            },
        )


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(argv)
    try:
        accept(
            arguments.installer,
            arguments.expected_version,
            arguments.report,
            arguments.launch_timeout_seconds,
        )
    except (OSError, subprocess.SubprocessError, WindowsInstallerAcceptanceFailure) as error:
        print(f"Windows 安装器验收失败：{error}", file=sys.stderr)
        return 1
    print(f"Windows 安装器验收通过：{arguments.report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
