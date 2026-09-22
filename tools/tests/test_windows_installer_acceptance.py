from __future__ import annotations

from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools.windows_installer_acceptance import (
    WindowsInstallerAcceptanceFailure,
    acceptance_environment,
    ensure_clean_install_registration,
    installed_desktop_version,
    verify_cli_version,
    locate_installed_layout,
    pe_subsystem,
    verify_desktop_is_windowless,
    wait_for_install_files_removed,
    write_new_report,
)


ROOT = Path(__file__).resolve().parents[2]
INSTALLER_HOOKS = ROOT / "apps" / "desktop" / "src-tauri" / "windows" / "hooks.nsh"
DESKTOP_MAIN = ROOT / "apps" / "desktop" / "src-tauri" / "src" / "main.rs"


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


if __name__ == "__main__":
    unittest.main()
