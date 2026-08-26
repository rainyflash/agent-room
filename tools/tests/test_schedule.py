from __future__ import annotations

from pathlib import Path
import shutil
import tempfile
import unittest

from tools.prodops.config import load_deployment_config
from tools.prodops.render import DeploymentPaths
from tools.prodops.schedule import BackupScheduleError, BackupScheduleInstaller


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
