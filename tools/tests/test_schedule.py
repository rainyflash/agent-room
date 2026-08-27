from __future__ import annotations

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tools.prodops.config import load_deployment_config
from tools.prodops.render import DeploymentPaths
from tools.prodops.schedule import (
    BackupScheduleError,
    BackupScheduleInstaller,
    _systemd_path,
)


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"


class BackupScheduleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.config_path = root / "配置 含空格.json"
        shutil.copyfile(EXAMPLE, self.config_path)
        self.config = load_deployment_config(self.config_path)
        self.paths = DeploymentPaths.from_state(root / "状态 含空格")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_rendered_timer_enforces_configured_rpo(self) -> None:
        files = BackupScheduleInstaller(self.config, self.paths, self.config_path).render()

        self.assertEqual(files.timer_name, "agent-room-backup.timer")
        self.assertIn("OnCalendar=*:0/15", files.timer_content)
        self.assertIn("AccuracySec=1us", files.timer_content)
        self.assertIn("Persistent=true", files.timer_content)
        self.assertIn(f"Unit={files.service_name}", files.timer_content)
        self.assertIn(f'"{self.config_path.as_posix()}"', files.service_content)
        self.assertIn(f'"{self.paths.state.as_posix()}"', files.service_content)
        self.assertNotIn("/bin/sh", files.service_content)
        self.assertNotIn('WorkingDirectory="', files.service_content)

    def test_systemd_path_uses_unit_escaping_without_quotes(self) -> None:
        self.assertEqual(_systemd_path("/opt/Agent Room/%release"), "/opt/Agent\\x20Room/%%release")

    @unittest.skipUnless(
        sys.platform.startswith("linux") and shutil.which("systemd-analyze"),
        "需要 Linux systemd-analyze",
    )
    def test_generated_units_pass_systemd_parser(self) -> None:
        files = BackupScheduleInstaller(self.config, self.paths, self.config_path).render()
        service = Path(self.temporary.name) / files.service_name
        timer = Path(self.temporary.name) / files.timer_name
        service.write_text(files.service_content, encoding="utf-8")
        timer.write_text(files.timer_content, encoding="utf-8")

        result = subprocess.run(
            ["systemd-analyze", "verify", str(service), str(timer)],
            check=False,
            text=True,
            encoding="utf-8",
            errors="replace",
            capture_output=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)

    def test_generated_units_are_stable_and_contain_no_secrets(self) -> None:
        installer = BackupScheduleInstaller(self.config, self.paths, self.config_path)

        first = installer.write_generated()
        second = installer.write_generated()

        self.assertEqual(first, second)
        for path in first:
            content = path.read_text(encoding="utf-8")
            self.assertNotIn("password", content.casefold())
            self.assertNotIn("token", content.casefold())

    def test_missing_config_is_rejected(self) -> None:
        missing = self.config_path.with_name("missing.json")

        with self.assertRaisesRegex(BackupScheduleError, "配置不存在"):
            BackupScheduleInstaller(self.config, self.paths, missing).render()


if __name__ == "__main__":
    unittest.main()
