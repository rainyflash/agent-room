from __future__ import annotations

import hashlib
from pathlib import Path
import tempfile
import unittest
import zipfile

from tools.closed_test import (
    desktop_suffix,
    safe_package_directory,
    write_reproducible_zip,
)


class ClosedTestPackagingTests(unittest.TestCase):
    def test_reproducible_zip_has_stable_digest_and_order(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / "b.txt").write_text("二", encoding="utf-8")
            (source / "a.txt").write_text("一", encoding="utf-8")
            first = root / "first.zip"
            second = root / "second.zip"

            write_reproducible_zip(first, (source / "b.txt", source / "a.txt"), source)
            write_reproducible_zip(second, (source / "a.txt", source / "b.txt"), source)

            self.assertEqual(hashlib.sha256(first.read_bytes()).digest(), hashlib.sha256(second.read_bytes()).digest())
            with zipfile.ZipFile(first) as bundle:
                self.assertEqual(bundle.namelist(), ["a.txt", "b.txt"])

    def test_reproducible_zip_rejects_file_outside_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            outside = root / "outside.txt"
            outside.write_text("拒绝", encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "根目录之外"):
                write_reproducible_zip(root / "archive.zip", (outside,), source)

    def test_platform_directory_rejects_traversal(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "平台标签"):
            safe_package_directory("../escape")

    def test_desktop_suffix_handles_compound_archive(self) -> None:
        self.assertEqual(desktop_suffix(Path("AgentRoom.tar.gz")), ".tar.gz")
        self.assertEqual(desktop_suffix(Path("AgentRoom.MSI")), ".msi")


if __name__ == "__main__":
    unittest.main()
