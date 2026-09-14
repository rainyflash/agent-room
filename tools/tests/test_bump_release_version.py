from __future__ import annotations

import json
import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools.bump_release_version import (
    JSON_VERSION_FILES,
    TEXT_VERSION_FILES,
    VersionBumpFailure,
    bump,
    workspace_version,
)


class BumpReleaseVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        for relative in JSON_VERSION_FILES:
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                json.dumps({"name": relative.stem, "version": "0.1.0-alpha.1"}) + "\n",
                encoding="utf-8",
            )
        for relative in TEXT_VERSION_FILES:
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            content = "current=0.1.0-alpha.1\n"
            if relative == Path("Cargo.toml"):
                content = (
                    "[workspace]\n[workspace.package]\nversion = \"0.1.0-alpha.1\"\n"
                    "[workspace.dependencies]\nexample = { version = \"0.1.0-alpha.1\" }\n"
                )
            path.write_text(content, encoding="utf-8")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_一次更新全部受管入口(self) -> None:
        old = bump(self.root, "0.1.0-alpha.2", refresh_lock=False)

        self.assertEqual(old, "0.1.0-alpha.1")
        self.assertEqual(workspace_version(self.root), "0.1.0-alpha.2")
        for relative in (*JSON_VERSION_FILES, *TEXT_VERSION_FILES):
            content = (self.root / relative).read_text(encoding="utf-8")
            self.assertNotIn("0.1.0-alpha.1", content)
            self.assertIn("0.1.0-alpha.2", content)

    def test_拒绝版本已经漂移的_json(self) -> None:
        path = self.root / JSON_VERSION_FILES[-1]
        path.write_text('{"version":"9.9.9"}\n', encoding="utf-8")

        with self.assertRaisesRegex(VersionBumpFailure, "不一致"):
            bump(self.root, "0.1.0-alpha.2", refresh_lock=False)
        self.assertEqual(workspace_version(self.root), "0.1.0-alpha.1")

    def test_json_升版只替换版本且保留既有格式(self) -> None:
        path = self.root / JSON_VERSION_FILES[0]
        source = (
            "{\n"
            '  "name": "desktop",\n'
            '  "version": "0.1.0-alpha.1",\n'
            '  "keywords": ["agent", "room"]\n'
            "}\n"
        )
        path.write_text(source, encoding="utf-8")

        bump(self.root, "0.1.0-alpha.2", refresh_lock=False)

        self.assertEqual(
            path.read_text(encoding="utf-8"),
            source.replace("0.1.0-alpha.1", "0.1.0-alpha.2"),
        )

    def test_拒绝非法版本(self) -> None:
        with self.assertRaisesRegex(VersionBumpFailure, "SemVer"):
            bump(self.root, "alpha two", refresh_lock=False)

    def test_升版后的许可证摘要对应新锁文件(self) -> None:
        lock = self.root / "Cargo.lock"
        notice = self.root / "THIRD_PARTY_NOTICES.md"
        lock.write_text("old lock", encoding="utf-8")
        notice.write_text("old digest", encoding="utf-8")

        def run(command, **kwargs):
            if command[0] == "cargo":
                lock.write_text(workspace_version(self.root), encoding="utf-8")
            else:
                notice.write_text(hashlib.sha256(lock.read_bytes()).hexdigest(), encoding="utf-8")
            return subprocess.CompletedProcess(command, 0)

        with patch("tools.bump_release_version.subprocess.run", side_effect=run):
            bump(self.root, "0.1.0-alpha.2")

        self.assertEqual(lock.read_text(encoding="utf-8"), "0.1.0-alpha.2")
        self.assertEqual(notice.read_text(encoding="utf-8"), hashlib.sha256(lock.read_bytes()).hexdigest())

    def test_许可证生成失败不能报告升版成功(self) -> None:
        with patch("tools.bump_release_version.subprocess.run", side_effect=[
            subprocess.CompletedProcess(["cargo"], 0),
            subprocess.CompletedProcess(["license_inventory.py"], 1),
        ]):
            with self.assertRaisesRegex(VersionBumpFailure, "许可证清单刷新失败"):
                bump(self.root, "0.1.0-alpha.2")


if __name__ == "__main__":
    unittest.main()
