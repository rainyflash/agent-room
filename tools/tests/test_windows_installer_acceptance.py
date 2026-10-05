from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, call, patch

from tools.windows_installer_acceptance import (
    IMAGE_RELEASE_DELAY_SECONDS,
    INSTALLER_MARKER,
    ExecutableImageHold,
    RunningRuntime,
    WindowsInstallerAcceptanceFailure,
    acceptance_environment,
    desktop_relaunch_problem,
    ensure_clean_install_registration,
    installed_desktop_version,
    mark_runtime_as_previous_build,
    marker_problem,
    runtime_digests,
    upgrade_while_image_is_released_late,
    verify_cli_version,
    verify_install_aborts_while_image_is_held,
    verify_runtime_digests,
    locate_installed_layout,
    pe_subsystem,
    verify_desktop_is_windowless,
    wait_for_install_files_removed,
    write_new_report,
)


ROOT = Path(__file__).resolve().parents[2]
INSTALLER_HOOKS = ROOT / "apps" / "desktop" / "src-tauri" / "windows" / "hooks.nsh"
DESKTOP_MAIN = ROOT / "apps" / "desktop" / "src-tauri" / "src" / "main.rs"
RUNTIME_IMAGES = ("agent-room-desktop.exe", "agent-room-bridge.exe", "agent-room-mcp.exe", "agent-room.exe")


def write_layout(root: Path) -> None:
    for filename in (*RUNTIME_IMAGES, "uninstall.exe"):
        root.joinpath(filename).write_bytes(f"binary {filename}".encode())


def portable_executable(subsystem: int, *, magic: int = 0x20B) -> bytes:
    """拼一个只有头部的 PE 文件：DOS 头、PE 签名、COFF 头和可选头。"""

    pe_offset = 128
    dos_header = b"MZ" + bytes(58) + pe_offset.to_bytes(4, "little")
    dos_header += bytes(pe_offset - len(dos_header))
    coff_header = bytes(16) + (112).to_bytes(2, "little") + bytes(2)
    optional_header = magic.to_bytes(2, "little") + bytes(66) + subsystem.to_bytes(2, "little") + bytes(42)
    return dos_header + b"PE\0\0" + coff_header + optional_header


class WindowsInstallerAcceptanceTests(unittest.TestCase):
    def test_cli_must_start_and_match_the_desktop_version(self) -> None:
        command = ("agent-room.exe", "--version")
        for output, code in (("agent-room 0.1.0-alpha.27\n", 0), ("agent-room 0.1.0-alpha.26\n", 0), ("", 1)):
            with self.subTest(output=output, code=code), patch(
                "tools.windows_installer_acceptance.subprocess.run",
                return_value=subprocess.CompletedProcess(command, code, output, ""),
            ):
                if code == 0 and "alpha.27" in output:
                    verify_cli_version(Path("agent-room.exe"), "0.1.0-alpha.27")
                else:
                    with self.assertRaises(WindowsInstallerAcceptanceFailure):
                        verify_cli_version(Path("agent-room.exe"), "0.1.0-alpha.27")

    def test_installer_hooks_stop_every_owned_runtime_before_write_and_delete(self) -> None:
        source = INSTALLER_HOOKS.read_text(encoding="utf-8")

        self.assertIn("!macro NSIS_HOOK_PREINSTALL", source)
        self.assertIn("!macro NSIS_HOOK_PREUNINSTALL", source)
        self.assertEqual(source.count("!insertmacro AGENT_ROOM_STOP_RUNTIME"), 2)
        self.assertIn('$SYSDIR\\taskkill.exe" /IM agent-room-desktop.exe', source)
        self.assertIn('$SYSDIR\\taskkill.exe" /IM agent-room-bridge.exe', source)
        self.assertIn('$SYSDIR\\taskkill.exe" /IM agent-room-mcp.exe', source)
        self.assertIn('$SYSDIR\\taskkill.exe" /IM agent-room.exe', source)
        self.assertIn("Push $0", source)
        self.assertGreaterEqual(source.count("Pop $0"), 5)

    def test_installer_hooks_wait_until_runtime_is_gone_and_files_are_writable(self) -> None:
        source = INSTALLER_HOOKS.read_text(encoding="utf-8")

        # 固定睡一会儿就写文件，桌面端退出得慢时会被 NSIS 静默跳过，留下新旧混装。
        self.assertNotIn("Sleep 750", source)
        for image in RUNTIME_IMAGES:
            self.assertIn(f'!insertmacro AGENT_ROOM_KILL_IF_RUNNING "{image}"', source)
            self.assertIn(f'!insertmacro AGENT_ROOM_MARK_IF_LOCKED "$INSTDIR\\{image}"', source)
        self.assertIn("nsis_tauri_utils::FindProcessCurrentUser", source)
        self.assertIn("nsis_tauri_utils::KillProcessCurrentUser", source)
        # 与 File 同样的写权限和共享方式，只开已有文件；只把共享冲突和锁冲突当作仍被占用。
        self.assertIn("kernel32::CreateFileW(w r3, i 0x40000000, i 1, p 0, i 3,", source)
        self.assertIn("${If} $3 = 32", source)
        self.assertIn("${OrIf} $3 = 33", source)
        self.assertIn("!define AGENT_ROOM_STOP_TIMEOUT_MS 20000", source)
        self.assertIn("kernel32::GetTickCount", source)

    def test_installer_hooks_fail_loudly_instead_of_skipping_files(self) -> None:
        source = INSTALLER_HOOKS.read_text(encoding="utf-8")

        # 必须写在所有宏之前：模板 !include 本文件后，这一行才对模板里的 File 生效。
        self.assertLess(source.index("AllowSkipFiles off"), source.index("!macro"))
        self.assertIn("MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION", source)
        self.assertIn("/SD IDCANCEL IDRETRY", source)
        abort = source[source.index("!macro AGENT_ROOM_ABORT_STILL_RUNNING"):]
        abort = abort[: abort.index("!macroend")]
        self.assertIn("${If} ${Silent}", abort)
        self.assertIn('Abort "${AGENT_ROOM_STILL_RUNNING_STOPPED}"', abort)
        # 中止前把保存的寄存器还回去，和正常路径一样四进四出。
        self.assertEqual(
            [line.strip() for line in abort.splitlines() if line.strip().startswith("Pop ")],
            ["Pop $3", "Pop $2", "Pop $1", "Pop $0"],
        )
        prompt = source[source.index("!define AGENT_ROOM_STILL_RUNNING_PROMPT"):].splitlines()[0]
        self.assertIn("Agent Room is still running", prompt)
        self.assertIn("Agent Room 仍在运行", prompt)

    def test_acceptance_environment_isolates_runtime_state(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            with patch.dict(
                "os.environ",
                {
                    "ACCEPTANCE_KEEP_ME": "kept",
                    "AGENT_ROOM_AGENT_ID": "must-not-leak",
                    "AGENT_ROOM_BRIDGE_DATA_DIR": "must-be-replaced",
                },
                clear=True,
            ):
                environment = acceptance_environment(root)

            self.assertEqual(environment["ACCEPTANCE_KEEP_ME"], "kept")
            self.assertNotIn("AGENT_ROOM_AGENT_ID", environment)
            self.assertEqual(
                environment["AGENT_ROOM_BRIDGE_DATA_DIR"],
                str(root / "bridge-data"),
            )
            self.assertRegex(
                environment["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"],
                r"^dev\.agent-room\.acceptance\.[0-9a-f]{32}$",
            )

    def test_layout_requires_all_same_directory_runtime_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for filename in (
                "agent-room-desktop.exe",
                "agent-room-bridge.exe",
                "agent-room-mcp.exe",
                "agent-room.exe",
                "uninstall.exe",
            ):
                root.joinpath(filename).write_bytes(b"binary")

            layout = locate_installed_layout(root)

            self.assertEqual(layout.root, root.resolve())
            self.assertEqual(layout.mcp.name, "agent-room-mcp.exe")
            self.assertEqual(layout.cli.name, "agent-room.exe")

    def test_layout_rejects_duplicate_runtime_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for filename in (
                "agent-room-desktop.exe",
                "agent-room-bridge.exe",
                "agent-room-mcp.exe",
                "agent-room.exe",
                "uninstall.exe",
            ):
                root.joinpath(filename).write_bytes(b"binary")
            duplicate = root / "duplicate"
            duplicate.mkdir()
            duplicate.joinpath("agent-room-mcp.exe").write_bytes(b"binary")

            with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "数量异常"):
                locate_installed_layout(root)

    def test_report_is_append_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            write_new_report(report, {"schemaVersion": 1, "result": "passed"})

            with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "拒绝覆盖"):
                write_new_report(report, {"schemaVersion": 1, "result": "changed"})

    def test_clean_install_rejects_existing_registration(self) -> None:
        with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "拒绝覆盖"):
            ensure_clean_install_registration(lambda _key: True)

        ensure_clean_install_registration(lambda _key: False)

    def test_desktop_must_be_linked_as_a_window_program(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, subsystem, magic in (("gui.exe", 2, 0x20B), ("gui32.exe", 2, 0x10B), ("console.exe", 3, 0x20B)):
                (root / name).write_bytes(portable_executable(subsystem, magic=magic))
            (root / "text.exe").write_bytes(b"not an executable at all")

            self.assertEqual(pe_subsystem(root / "gui.exe"), 2)
            self.assertEqual(pe_subsystem(root / "gui32.exe"), 2)
            self.assertEqual(pe_subsystem(root / "console.exe"), 3)
            verify_desktop_is_windowless(root / "gui.exe")
            # 按控制台子系统链接的桌面端每次启动都会先弹一个命令行窗口，候选不得带着它发布。
            with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "控制台窗口"):
                verify_desktop_is_windowless(root / "console.exe")
            with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "不是 Windows 可执行文件"):
                verify_desktop_is_windowless(root / "text.exe")

    def test_desktop_source_hides_the_console_in_release_builds(self) -> None:
        # 安装器验收只在候选上跑；这里在 PR 阶段就拦住把这一行删掉的改动。
        source = DESKTOP_MAIN.read_text(encoding="utf-8")
        self.assertIn('#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]', source)

    def test_installed_version_uses_headless_desktop_probe(self) -> None:
        completed = type(
            "Completed",
            (),
            {"returncode": 0, "stdout": "0.1.0-alpha.4\n", "stderr": ""},
        )()
        with patch("tools.windows_installer_acceptance.subprocess.run", return_value=completed) as run:
            version = installed_desktop_version(Path("agent-room-desktop.exe"))

        self.assertEqual(version, "0.1.0-alpha.4")
        self.assertEqual(
            run.call_args.args[0],
            ("agent-room-desktop.exe", "--installer-version"),
        )

    def test_uninstall_accepts_empty_directories_but_rejects_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.joinpath("empty").mkdir()
            wait_for_install_files_removed(root, timeout_seconds=1)

            root.joinpath("residual.exe").write_bytes(b"binary")
            with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "residual.exe"):
                wait_for_install_files_removed(root, timeout_seconds=1)

    def test_previous_build_marker_changes_every_runtime_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_layout(root)
            layout = locate_installed_layout(root)
            payload = runtime_digests(layout)
            installed = {name: root.joinpath(name).read_bytes() for name in RUNTIME_IMAGES}

            previous = mark_runtime_as_previous_build(layout)

            self.assertEqual(set(previous), set(RUNTIME_IMAGES))
            self.assertTrue(all(previous[name] != payload[name] for name in RUNTIME_IMAGES))
            verify_runtime_digests(layout, previous, "标记后")

            # 复现混装：安装器换掉了三个 sidecar，却跳过了还被占着的桌面端。
            for name in RUNTIME_IMAGES[1:]:
                root.joinpath(name).write_bytes(installed[name])
            with self.assertRaises(WindowsInstallerAcceptanceFailure) as failure:
                verify_runtime_digests(layout, payload, "运行中原地升级后")
            self.assertIn("agent-room-desktop.exe", str(failure.exception))
            self.assertNotIn("agent-room-bridge.exe", str(failure.exception))

    @unittest.skipUnless(os.name == "nt", "映像占用只在 Windows 上有意义")
    def test_image_hold_blocks_writes_until_released(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "held.exe"
            shutil.copyfile(sys.executable, executable)

            with ExecutableImageHold(executable) as hold:
                with self.assertRaises(PermissionError):
                    executable.open("r+b")
                hold.release()
                with executable.open("r+b"):
                    pass

    def test_install_must_abort_while_desktop_image_is_held(self) -> None:
        layout = MagicMock()
        install = ("setup.exe", "/S")
        unchanged = {"agent-room-desktop.exe": "previous"}
        for code, after, message in (
            (0, unchanged, "退出码 0"),
            (2, {"agent-room-desktop.exe": "replaced"}, "agent-room-desktop.exe"),
        ):
            with self.subTest(code=code), patch(
                "tools.windows_installer_acceptance.ExecutableImageHold"
            ), patch(
                "tools.windows_installer_acceptance.run_captured",
                return_value=subprocess.CompletedProcess(install, code, "", ""),
            ), patch("tools.windows_installer_acceptance.runtime_digests", return_value=after):
                with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, message):
                    verify_install_aborts_while_image_is_held(install, layout, unchanged)

        with patch("tools.windows_installer_acceptance.ExecutableImageHold") as hold, patch(
            "tools.windows_installer_acceptance.run_captured",
            return_value=subprocess.CompletedProcess(install, 2, "Agent Room is still running", ""),
        ) as run, patch("tools.windows_installer_acceptance.runtime_digests", return_value=unchanged):
            verify_install_aborts_while_image_is_held(install, layout, unchanged)

        hold.assert_called_once_with(layout.desktop)
        run.assert_called_once_with(install, timeout_seconds=300)

    def run_late_upgrade(self, relaunch: str | None, marker: str | None) -> tuple[MagicMock, MagicMock, MagicMock]:
        steps = MagicMock()
        steps.relaunch.return_value = relaunch
        steps.marker.return_value = marker
        runtime = RunningRuntime(desktop=MagicMock(), mcp=MagicMock(), bridge_pid=42)
        layout = MagicMock()
        upgrade = MagicMock()
        upgrade.poll.return_value = 0
        steps.start.return_value = upgrade
        steps.hold.return_value.__enter__.return_value = steps.held
        with patch("tools.windows_installer_acceptance.ExecutableImageHold", steps.hold), patch(
            "tools.windows_installer_acceptance.start_captured", steps.start
        ), patch("tools.windows_installer_acceptance.wait_for_process_exit", steps.wait_exit), patch(
            "tools.windows_installer_acceptance.time.sleep", steps.sleep
        ), patch("tools.windows_installer_acceptance.finish_checked", steps.finish), patch(
            "tools.windows_installer_acceptance.wait_for_runtime_exit", steps.wait_runtime
        ), patch("tools.windows_installer_acceptance.desktop_relaunch_problem", steps.relaunch), patch(
            "tools.windows_installer_acceptance.marker_problem", steps.marker
        ):
            upgrade_while_image_is_released_late(("setup.exe", "/S"), layout, runtime)
        return steps, layout, upgrade

    def test_running_upgrade_releases_the_image_only_after_the_desktop_is_gone(self) -> None:
        steps, layout, upgrade = self.run_late_upgrade(None, None)

        runtime = steps.wait_runtime.call_args.args[0]
        self.assertEqual(
            [entry for entry in steps.mock_calls if not entry[0].startswith(("hold().", "start()."))],
            [
                call.hold(layout.desktop),
                call.start(("setup.exe", "/S")),
                call.wait_exit(runtime.desktop, "桌面端"),
                call.sleep(IMAGE_RELEASE_DELAY_SECONDS),
                # 安装器还在等映像放开：这时看 Agent 拉不拉得起桌面端、标记是否被安装器独占。
                call.relaunch(layout.desktop),
                call.marker(layout.root / INSTALLER_MARKER),
                call.held.release(),
                call.finish(upgrade, "运行中原地升级", timeout_seconds=300),
                call.wait_runtime(runtime, ""),
            ],
        )
        upgrade.kill.assert_not_called()

    def test_running_upgrade_fails_when_the_desktop_could_start_midway(self) -> None:
        with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "运行中原地升级时：桌面端还能启动；没有标记"):
            self.run_late_upgrade("桌面端还能启动", "没有标记")

    def test_desktop_that_cannot_be_started_from_the_install_directory_is_fine(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertIsNone(desktop_relaunch_problem(Path(directory) / "agent-room-desktop.exe"))

    def test_desktop_that_still_starts_is_reported_and_stopped_again(self) -> None:
        launched = MagicMock()
        with patch("tools.windows_installer_acceptance.subprocess.Popen", return_value=launched):
            problem = desktop_relaunch_problem(Path("install") / "agent-room-desktop.exe")

        self.assertIn("桌面端还能从安装目录启动", problem or "")
        launched.kill.assert_called_once_with()
        launched.wait.assert_called_once_with(timeout=30)

    def test_marker_must_exist_and_be_held_by_the_installer_alone(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / INSTALLER_MARKER
            self.assertIn("没有标记", marker_problem(marker) or "")
            marker.write_bytes(b"")
            self.assertIn("没有被安装器独占", marker_problem(marker) or "")
            with patch.object(Path, "open", side_effect=PermissionError(13, "sharing violation")):
                self.assertIsNone(marker_problem(marker))


if __name__ == "__main__":
    unittest.main()
