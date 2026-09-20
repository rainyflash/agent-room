from __future__ import annotations

from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools.macos_bundle_acceptance import (
    MacosBundleAcceptanceFailure,
    accept,
    acceptance_environment,
    installed_desktop_version,
    locate_app,
    locate_installed_layout,
    stop_managed_bridge,
    verify_cli_version,
    write_new_report,
)


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "macos.yml"
RUNTIME_EXECUTABLES = (
    "agent-room-desktop",
    "agent-room-bridge",
    "agent-room-mcp",
    "agent-room",
)


def build_application(root: Path, name: str = "Agent Room.app") -> Path:
    app = root / name
    executables = app / "Contents" / "MacOS"
    executables.mkdir(parents=True)
    for executable in RUNTIME_EXECUTABLES:
        path = executables / executable
        path.write_bytes(b"binary")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return app


class MacosBundleAcceptanceTests(unittest.TestCase):
    def test_layout_requires_every_runtime_executable_inside_the_app(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = build_application(Path(directory))

            layout = locate_installed_layout(app)

            self.assertEqual(layout.app, app)
            self.assertEqual(layout.desktop.parent.name, "MacOS")
            self.assertEqual(layout.cli.name, "agent-room")

            layout.mcp.unlink()
            with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "agent-room-mcp"):
                locate_installed_layout(app)

    def test_image_must_contain_exactly_one_application(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            mountpoint = Path(directory)
            with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "数量异常"):
                locate_app(mountpoint)

            app = build_application(mountpoint)
            self.assertEqual(locate_app(mountpoint), app)

            build_application(mountpoint, "Agent Room Beta.app")
            with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "数量异常"):
                locate_app(mountpoint)

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
            self.assertEqual(environment["AGENT_ROOM_BRIDGE_DATA_DIR"], str(root / "bridge-data"))
            self.assertRegex(
                environment["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"],
                r"^dev\.agent-room\.acceptance\.[0-9a-f]{32}$",
            )

    def test_cli_must_start_and_match_the_desktop_version(self) -> None:
        command = ("agent-room", "--version")
        for output, code, accepted in (
            ("agent-room 0.1.0-alpha.45\n", 0, True),
            ("agent-room 0.1.0-alpha.44\n", 0, False),
            ("", 1, False),
        ):
            with self.subTest(output=output, code=code), patch(
                "tools.macos_bundle_acceptance.subprocess.run",
                return_value=subprocess.CompletedProcess(command, code, output, ""),
            ):
                if accepted:
                    verify_cli_version(Path("agent-room"), "0.1.0-alpha.45")
                else:
                    with self.assertRaises(MacosBundleAcceptanceFailure):
                        verify_cli_version(Path("agent-room"), "0.1.0-alpha.45")

    def test_installed_version_uses_headless_desktop_probe(self) -> None:
        completed = subprocess.CompletedProcess(("desktop",), 0, "0.1.0-alpha.45\n", "")
        with patch(
            "tools.macos_bundle_acceptance.subprocess.run", return_value=completed
        ) as run:
            version = installed_desktop_version(Path("agent-room-desktop"))

        self.assertEqual(version, "0.1.0-alpha.45")
        self.assertEqual(
            run.call_args.args[0], ("agent-room-desktop", "--installer-version")
        )

    def test_report_is_append_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            write_new_report(report, {"schemaVersion": 1, "result": "passed"})

            with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "拒绝覆盖"):
                write_new_report(report, {"schemaVersion": 1, "result": "changed"})

    def test_a_vanished_managed_bridge_fails_the_acceptance(self) -> None:
        with patch("tools.macos_bundle_acceptance.os.kill", side_effect=ProcessLookupError()):
            with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "受管 Bridge"):
                stop_managed_bridge(4321)

        with patch("tools.macos_bundle_acceptance.os.kill") as kill:
            stop_managed_bridge(4321)

        self.assertEqual(kill.call_args.args[0], 4321)

    def test_acceptance_refuses_anything_but_macos_and_non_semver(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "agent-room.dmg"
            image.write_bytes(b"image")
            report = Path(directory) / "report.json"

            with patch("tools.macos_bundle_acceptance.sys.platform", "win32"):
                with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "只能在 macOS"):
                    accept(image, "0.1.0-alpha.45", report, Path(directory), 120)
                with self.assertRaisesRegex(MacosBundleAcceptanceFailure, "SemVer"):
                    accept(image, "alpha.45", report, Path(directory), 120)

            self.assertFalse(report.exists())

    def test_workflow_accepts_the_built_disk_image(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("tools/macos_bundle_acceptance.py", workflow)
        self.assertIn("--expected-version", workflow)
        self.assertIn('--report "$RUNNER_TEMP/macos-bundle-acceptance.json"', workflow)


if __name__ == "__main__":
    unittest.main()
