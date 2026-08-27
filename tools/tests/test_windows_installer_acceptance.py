from __future__ import annotations

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.windows_installer_acceptance import (
    WindowsInstallerAcceptanceFailure,
    acceptance_environment,
    locate_installed_layout,
    wait_for_install_files_removed,
    write_new_report,
)


class WindowsInstallerAcceptanceTests(unittest.TestCase):
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
                "uninstall.exe",
            ):
                root.joinpath(filename).write_bytes(b"binary")

            layout = locate_installed_layout(root)

            self.assertEqual(layout.root, root.resolve())
            self.assertEqual(layout.mcp.name, "agent-room-mcp.exe")

    def test_layout_rejects_duplicate_runtime_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for filename in (
                "agent-room-desktop.exe",
                "agent-room-bridge.exe",
                "agent-room-mcp.exe",
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
