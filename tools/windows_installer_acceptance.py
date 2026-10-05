#!/usr/bin/env python3
"""在一次性目录中验收 Agent Room Windows NSIS 安装器。"""

from __future__ import annotations

import argparse
from collections.abc import Callable
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
import uuid


SCHEMA_VERSION: Final = 1
DESKTOP_EXECUTABLE: Final = "agent-room-desktop.exe"
BRIDGE_EXECUTABLE: Final = "agent-room-bridge.exe"
MCP_EXECUTABLE: Final = "agent-room-mcp.exe"
CLI_EXECUTABLE: Final = "agent-room.exe"
WINDOWS_GUI_SUBSYSTEM: Final = 2
SEMVER_PATTERN: Final = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
BRIDGE_STABILITY_SECONDS: Final = 2.0
PROCESS_STABILITY_SECONDS: Final = 1.0
PROCESS_EXIT_TIMEOUT_SECONDS: Final = 30
# 追加在已安装程序末尾，把它们变成“上一版”：PE 加载器不读文件尾部的附加数据，程序照常运行，
# 但字节和安装包里的不同。候选只有一个安装包、新旧版本号相同，只比版本号查不出文件有没有真的换掉。
PREVIOUS_BUILD_MARKER: Final = b"\0agent-room installer acceptance: previous build\0"
# NSIS 在安装段里中止时，静默安装的退出码。
INSTALLER_ABORTED_EXIT_CODE: Final = 2
# 旧钩子结束进程后只固定等 0.5 + 0.75 秒，模板再等 0.5 秒就写文件；映像多占 5 秒足以让那种写法漏掉
# 桌面端，又远短于钩子等待进程退出的 20 秒上限。
IMAGE_RELEASE_DELAY_SECONDS: Final = 5.0
LOAD_LIBRARY_AS_IMAGE_RESOURCE: Final = 0x20
# 换文件期间安装器在安装目录里独占的标记（hooks.nsh 的 AGENT_ROOM_INSTALLER_MARKER，桌面端 installer_marker.rs）。
INSTALLER_MARKER: Final = "installer-running.lock"
INSTALLER_REGISTRATION_KEYS: Final = (
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Agent Room",
    r"HKCU\Software\agent-room\Agent Room",
    r"HKCU\Software\Classes\agent-room",
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
    cli: Path
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
    cli = unique_file(root, CLI_EXECUTABLE)
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
    for path in (bridge, mcp, cli, uninstallers[0]):
        if path.parent.resolve() != executable_parent:
            raise WindowsInstallerAcceptanceFailure("桌面端、Bridge、MCP 与卸载器必须位于同一目录。")
    return InstalledLayout(executable_parent, desktop, bridge, mcp, cli, uninstallers[0])


def run_captured(command: Sequence[str], *, timeout_seconds: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout_seconds,
    )


def ensure_succeeded(completed: subprocess.CompletedProcess[str], label: str) -> None:
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        raise WindowsInstallerAcceptanceFailure(
            f"{label}失败（退出码 {completed.returncode}）：{detail}"
        )


def run_checked(command: Sequence[str], label: str, *, timeout_seconds: int) -> None:
    ensure_succeeded(run_captured(command, timeout_seconds=timeout_seconds), label)


def start_captured(command: Sequence[str]) -> subprocess.Popen[str]:
    return subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )


def finish_checked(process: subprocess.Popen[str], label: str, *, timeout_seconds: int) -> None:
    try:
        stdout, stderr = process.communicate(timeout=timeout_seconds)
    except subprocess.TimeoutExpired as error:
        process.kill()
        process.communicate()
        raise WindowsInstallerAcceptanceFailure(f"{label}超时。") from error
    ensure_succeeded(subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr), label)


def windows_registry_key_exists(key: str) -> bool:
    completed = subprocess.run(
        ("reg", "query", key),
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=30,
    )
    if completed.returncode not in (0, 1):
        raise WindowsInstallerAcceptanceFailure("无法确认 Windows 安装注册状态。")
    return completed.returncode == 0


def ensure_clean_install_registration(
    key_exists: Callable[[str], bool] | None = None,
) -> None:
    query = key_exists or windows_registry_key_exists
    occupied = tuple(key for key in INSTALLER_REGISTRATION_KEYS if query(key))
    if occupied:
        raise WindowsInstallerAcceptanceFailure(
            "干净安装验收拒绝覆盖当前用户已有的 Agent Room 安装注册；"
            "请在一次性 Windows runner 上运行。"
        )


def pe_subsystem(executable: Path) -> int:
    """读取 PE 可选头里的子系统编号（2 = 窗口程序，3 = 控制台程序）。"""
    with executable.open("rb") as handle:
        dos_header = handle.read(64)
        if len(dos_header) < 64 or dos_header[:2] != b"MZ":
            raise WindowsInstallerAcceptanceFailure(f"{executable.name} 不是 Windows 可执行文件。")
        pe_offset = int.from_bytes(dos_header[60:64], "little")
        handle.seek(pe_offset)
        # PE 签名 4 字节 + COFF 头 20 字节；PE32 与 PE32+ 的 Subsystem 都在可选头偏移 68 处。
        headers = handle.read(24 + 70)
    if len(headers) < 94 or headers[:4] != b"PE\0\0":
        raise WindowsInstallerAcceptanceFailure(f"{executable.name} 的 PE 头无效。")
    return int.from_bytes(headers[92:94], "little")


def verify_desktop_is_windowless(desktop: Path) -> None:
    # 桌面端必须按窗口子系统链接；按控制台子系统链接的话，每次启动都会先弹一个命令行窗口。
    subsystem = pe_subsystem(desktop)
    if subsystem != WINDOWS_GUI_SUBSYSTEM:
        raise WindowsInstallerAcceptanceFailure(
            f"已安装桌面端按子系统 {subsystem} 链接，启动会带控制台窗口；应为窗口程序（{WINDOWS_GUI_SUBSYSTEM}）。"
        )


def installed_desktop_version(desktop: Path, *, timeout_seconds: int = 30) -> str:
    completed = subprocess.run(
        (str(desktop), "--installer-version"),
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout_seconds,
    )
    version = completed.stdout.strip()
    if completed.returncode != 0 or SEMVER_PATTERN.fullmatch(version) is None:
        detail = (completed.stderr or completed.stdout).strip()
        raise WindowsInstallerAcceptanceFailure(
            f"无法读取已安装桌面端版本：{detail or '无有效输出'}"
        )
    return version


def verify_cli_version(cli: Path, expected_version: str) -> None:
    completed = subprocess.run((str(cli), "--version"), check=False, capture_output=True,
                               text=True, encoding="utf-8", timeout=30)
    if completed.returncode != 0 or completed.stdout.strip() != f"agent-room {expected_version}":
        raise WindowsInstallerAcceptanceFailure("已安装 CLI 无法启动或版本与桌面不一致。")


def verify_installed_versions(layout: InstalledLayout, expected_version: str, stage: str) -> None:
    verify_desktop_is_windowless(layout.desktop)
    verify_cli_version(layout.cli, expected_version)
    actual_version = installed_desktop_version(layout.desktop)
    if actual_version != expected_version:
        raise WindowsInstallerAcceptanceFailure(
            f"{stage}桌面端版本 {actual_version}，预期 {expected_version}。"
        )


def runtime_files(layout: InstalledLayout) -> tuple[Path, ...]:
    return (layout.desktop, layout.bridge, layout.mcp, layout.cli)


def runtime_digests(layout: InstalledLayout) -> dict[str, str]:
    return {path.name: sha256_file(path) for path in runtime_files(layout)}


def mark_runtime_as_previous_build(layout: InstalledLayout) -> dict[str, str]:
    """给四个已安装程序追加标记，当作上一版；返回标记后的摘要。"""
    installed = runtime_digests(layout)
    for path in runtime_files(layout):
        with path.open("ab") as target:
            target.write(PREVIOUS_BUILD_MARKER)
    marked = runtime_digests(layout)
    if any(marked[name] == digest for name, digest in installed.items()):
        raise WindowsInstallerAcceptanceFailure("无法把已安装程序标记为上一版。")
    return marked


def verify_runtime_digests(layout: InstalledLayout, expected: dict[str, str], stage: str) -> None:
    actual = runtime_digests(layout)
    mismatched = sorted(name for name, digest in expected.items() if actual.get(name) != digest)
    if mismatched:
        raise WindowsInstallerAcceptanceFailure(
            f"{stage}这些程序与预期内容不一致：{', '.join(mismatched)}。"
        )


class ExecutableImageHold:
    """在验收进程里把程序按映像映射一份，模拟“进程还没退干净，映像仍被占着”。

    和运行中的程序一样，占用期间谁也写不进这个文件；安装器结束不了验收进程，只有 release() 才放开。
    """

    def __init__(self, path: Path) -> None:
        import ctypes
        from ctypes import wintypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        load_library = kernel32.LoadLibraryExW
        load_library.argtypes = (wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD)
        load_library.restype = wintypes.HMODULE
        self._free_library = kernel32.FreeLibrary
        self._free_library.argtypes = (wintypes.HMODULE,)
        self._free_library.restype = wintypes.BOOL
        self._module = load_library(str(path), None, LOAD_LIBRARY_AS_IMAGE_RESOURCE)
        if not self._module:
            raise WindowsInstallerAcceptanceFailure(
                f"无法占用 {path.name} 的映像（错误 {ctypes.get_last_error()}）。"
            )
        try:
            with path.open("r+b"):
                pass
        except PermissionError:
            return
        except BaseException:
            self.release()
            raise
        self.release()
        raise WindowsInstallerAcceptanceFailure(f"占用 {path.name} 的映像后仍能写入，模拟不了运行中的程序。")

    def release(self) -> None:
        if self._module:
            self._free_library(self._module)
            self._module = None

    def __enter__(self) -> ExecutableImageHold:
        return self

    def __exit__(self, *_: object) -> None:
        self.release()


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
    bridge_pid: int | None = None
    observed_at: float | None = None
    while time.monotonic() < deadline:
        if desktop.poll() is not None:
            raise WindowsInstallerAcceptanceFailure(
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
    raise WindowsInstallerAcceptanceFailure("桌面端未在时限内启动受管 Bridge。")


def wait_for_process_stability(
    process: subprocess.Popen[bytes],
    label: str,
    *,
    stability_seconds: float = PROCESS_STABILITY_SECONDS,
) -> None:
    deadline = time.monotonic() + stability_seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise WindowsInstallerAcceptanceFailure(
                f"{label}未保持运行（退出码 {process.returncode}）。"
            )
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
        raise WindowsInstallerAcceptanceFailure(f"{label}没有按安装器要求退出。") from error


def wait_for_image_exit(
    image_name: str,
    process_id: int,
    label: str,
    *,
    timeout_seconds: int = PROCESS_EXIT_TIMEOUT_SECONDS,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if process_id not in process_ids(image_name):
            return
        time.sleep(0.5)
    raise WindowsInstallerAcceptanceFailure(f"{label}没有按安装器要求退出。")


def acceptance_environment(temporary: Path) -> dict[str, str]:
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("AGENT_ROOM_")
    }
    environment["AGENT_ROOM_BRIDGE_DATA_DIR"] = str(temporary / "bridge-data")
    environment["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"] = (
        f"dev.agent-room.acceptance.{uuid.uuid4().hex}"
    )
    return environment


def terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
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
    if process.stdin is not None:
        process.stdin.close()


@dataclass(frozen=True, slots=True)
class RunningRuntime:
    """从安装目录启动的桌面端（无界面验收模式）、它拉起的受管 Bridge 和宿主会启动的 MCP。"""

    desktop: subprocess.Popen[bytes]
    mcp: subprocess.Popen[bytes]
    bridge_pid: int


def launch_runtime(
    layout: InstalledLayout,
    environment: dict[str, str],
    launch_timeout_seconds: int,
    stage: str,
) -> RunningRuntime:
    previous_bridge_ids = process_ids(BRIDGE_EXECUTABLE)
    desktop = subprocess.Popen(
        (str(layout.desktop), "--installer-acceptance"),
        cwd=layout.root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=environment,
    )
    mcp: subprocess.Popen[bytes] | None = None
    try:
        bridge_pid = wait_for_bridge(previous_bridge_ids, desktop, launch_timeout_seconds)
        mcp = subprocess.Popen(
            (str(layout.mcp),),
            cwd=layout.root,
            stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env=environment,
        )
        wait_for_process_stability(mcp, f"{stage} MCP".lstrip())
    except BaseException:
        if mcp is not None:
            terminate_process_tree(mcp)
        terminate_process_tree(desktop)
        raise
    return RunningRuntime(desktop, mcp, bridge_pid)


def wait_for_runtime_exit(runtime: RunningRuntime, stage: str) -> None:
    wait_for_process_exit(runtime.desktop, f"{stage}桌面端")
    wait_for_process_exit(runtime.mcp, f"{stage} MCP".lstrip())
    wait_for_image_exit(BRIDGE_EXECUTABLE, runtime.bridge_pid, f"{stage}受管 Bridge")


def terminate_runtime(runtime: RunningRuntime) -> None:
    terminate_process_tree(runtime.mcp)
    terminate_process_tree(runtime.desktop)


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


def validated_installer(installer: Path, expected_version: str, launch_timeout_seconds: int) -> Path:
    if os.name != "nt":
        raise WindowsInstallerAcceptanceFailure("Windows 安装器验收只能在 Windows 上运行。")
    if not SEMVER_PATTERN.fullmatch(expected_version):
        raise WindowsInstallerAcceptanceFailure("expected-version 不是受支持的 SemVer。")
    installer = installer.resolve(strict=True)
    if not installer.is_file() or installer.suffix.lower() != ".exe":
        raise WindowsInstallerAcceptanceFailure("installer 必须是存在的 EXE 文件。")
    if launch_timeout_seconds < 5 or launch_timeout_seconds > 120:
        raise WindowsInstallerAcceptanceFailure("launch-timeout-seconds 必须在 5 到 120 之间。")
    return installer


def verify_install_aborts_while_image_is_held(
    install: Sequence[str],
    layout: InstalledLayout,
    unchanged: dict[str, str],
) -> None:
    """桌面端映像一直被占着：安装器必须中止，不许跳过写不进的文件、留下新旧混装。"""
    with ExecutableImageHold(layout.desktop):
        completed = run_captured(install, timeout_seconds=300)
    if completed.returncode != INSTALLER_ABORTED_EXIT_CODE:
        detail = (completed.stderr or completed.stdout).strip()
        raise WindowsInstallerAcceptanceFailure(
            f"桌面端映像一直被占用时安装器退出码 {completed.returncode}，"
            f"应中止并返回 {INSTALLER_ABORTED_EXIT_CODE}：{detail or '无输出'}"
        )
    verify_runtime_digests(layout, unchanged, "安装器中止后")


def desktop_relaunch_problem(desktop: Path) -> str | None:
    """像 Agent 的 MCP 和命令行那样从安装目录启动桌面端。钩子停 Agent Room 期间这一步必须失败。"""
    try:
        launched = subprocess.Popen(
            (str(desktop),),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except FileNotFoundError:
        return None
    launched.kill()
    launched.wait(timeout=30)
    return "钩子停 Agent Room 期间，桌面端还能从安装目录启动，Agent 会把旧版拉起来。"


def marker_problem(marker: Path) -> str | None:
    """换文件期间的标记要被安装器独占：桌面端打开它时碰上共享冲突，才知道安装器正在换文件。"""
    try:
        with marker.open("rb"):
            pass
    except FileNotFoundError:
        return "钩子停 Agent Room 期间没有标记，新版桌面端会在换文件的当口照常启动。"
    except PermissionError:
        return None
    return "标记没有被安装器独占，桌面端看不出安装器正在换文件。"


def upgrade_while_image_is_released_late(
    install: Sequence[str],
    layout: InstalledLayout,
    runtime: RunningRuntime,
) -> None:
    """运行中升级：桌面端被结束后映像又多占几秒才放开，安装器要一直等到能写再覆盖。

    安装器还在等的时候从外面看一眼：桌面端程序已被挪开、Agent 拉不起它，换文件期间的标记被安装器独占。
    """
    with ExecutableImageHold(layout.desktop) as hold:
        upgrade = start_captured(install)
        try:
            wait_for_process_exit(runtime.desktop, "桌面端")
            time.sleep(IMAGE_RELEASE_DELAY_SECONDS)
            found = (desktop_relaunch_problem(layout.desktop), marker_problem(layout.root / INSTALLER_MARKER))
            hold.release()
            finish_checked(upgrade, "运行中原地升级", timeout_seconds=300)
        finally:
            if upgrade.poll() is None:
                upgrade.kill()
                upgrade.communicate()
    wait_for_runtime_exit(runtime, "")
    problems = [problem for problem in found if problem is not None]
    if problems:
        raise WindowsInstallerAcceptanceFailure(f"运行中原地升级时：{'；'.join(problems)}")


def accept(installer: Path, expected_version: str, report: Path, launch_timeout_seconds: int) -> None:
    installer = validated_installer(installer, expected_version, launch_timeout_seconds)
    ensure_clean_install_registration()

    with tempfile.TemporaryDirectory(prefix="agent-room-installer-acceptance-") as temporary:
        install_root = Path(temporary) / "installed"
        install = (str(installer), "/S", "/NS", f"/D={install_root}")
        layout: InstalledLayout | None = None
        runtime: RunningRuntime | None = None
        environment = acceptance_environment(Path(temporary))
        try:
            run_checked(install, "静默安装", timeout_seconds=300)
            layout = locate_installed_layout(install_root)
            verify_installed_versions(layout, expected_version, "已安装")
            payload = runtime_digests(layout)
            previous_build = mark_runtime_as_previous_build(layout)

            runtime = launch_runtime(layout, environment, launch_timeout_seconds, "")
            verify_install_aborts_while_image_is_held(install, layout, previous_build)
            terminate_runtime(runtime)

            runtime = launch_runtime(layout, environment, launch_timeout_seconds, "")
            upgrade_while_image_is_released_late(install, layout, runtime)
            terminate_runtime(runtime)
            runtime = None

            layout = locate_installed_layout(install_root)
            verify_runtime_digests(layout, payload, "运行中原地升级后")
            verify_installed_versions(layout, expected_version, "原地升级后")

            runtime = launch_runtime(layout, environment, launch_timeout_seconds, "升级后的")
            run_checked((str(layout.uninstaller), "/S"), "运行中静默卸载", timeout_seconds=300)
            wait_for_runtime_exit(runtime, "卸载时的")
            terminate_runtime(runtime)
            runtime = None
        finally:
            if runtime is not None:
                terminate_runtime(runtime)
            if layout is not None and layout.uninstaller.is_file():
                run_checked((str(layout.uninstaller), "/S"), "静默卸载", timeout_seconds=300)

        wait_for_install_files_removed(install_root)

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
                    "desktopWindowless": True,
                    "bridgePresent": True,
                    "mcpPresent": True,
                    "desktopVersion": True,
                    "desktopLaunch": True,
                    "managedBridgeLaunch": True,
                    "mcpLaunch": True,
                    "lockedImageInstallAborted": True,
                    "lockedImageInstallLeftFilesUnchanged": True,
                    "runningUpgrade": True,
                    "upgradeStoppedDesktop": True,
                    "upgradeStoppedBridge": True,
                    "upgradeStoppedMcp": True,
                    "upgradeWaitedForImageRelease": True,
                    "upgradeKeptDesktopFromStarting": True,
                    "upgradeHeldInstallerMarker": True,
                    "upgradeReplacedRuntimeFiles": True,
                    "postUpgradeDesktopLaunch": True,
                    "postUpgradeBridgeLaunch": True,
                    "postUpgradeMcpLaunch": True,
                    "silentUninstall": True,
                    "uninstallStoppedRuntime": True,
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
