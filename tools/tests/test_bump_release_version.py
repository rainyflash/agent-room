from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
