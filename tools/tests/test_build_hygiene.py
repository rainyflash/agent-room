from __future__ import annotations

import subprocess
import unittest
from unittest.mock import MagicMock, call, patch

from tools.build_hygiene import (
    BuildHygieneFailure,
    ROOT,
    TAURI_SCHEMA_PATH,
    verify_and_clean_tauri_schemas,
)


class BuildHygieneTests(unittest.TestCase):
    @patch("tools.build_hygiene.subprocess.run")
    def test_只清理固定的_tauri_schema_目录(self, run: MagicMock) -> None:
        run.side_effect = [
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, stdout="", stderr=""),
        ]

        verify_and_clean_tauri_schemas()

        self.assertEqual(
            run.call_args_list,
            [
                call(
                    [
                        "git",
                        "diff",
                        "--exit-code",
                        "--",
                        ".",
                        f":(exclude){TAURI_SCHEMA_PATH}",
                    ],
                    cwd=ROOT,
                    check=False,
                    capture_output=False,
                    text=True,
                ),
                call(
                    ["git", "restore", "--source=HEAD", "--", TAURI_SCHEMA_PATH],
                    cwd=ROOT,
                    check=False,
                    capture_output=False,
                    text=True,
                ),
                call(
                    ["git", "clean", "-fd", "--", TAURI_SCHEMA_PATH],
                    cwd=ROOT,
                    check=False,
                    capture_output=False,
                    text=True,
                ),
                call(
                    ["git", "status", "--porcelain"],
                    cwd=ROOT,
                    check=False,
                    capture_output=True,
                    text=True,
                ),
            ],
        )

    @patch("tools.build_hygiene.subprocess.run")
    def test_构建改写_schema_之外源码时立即失败(self, run: MagicMock) -> None:
        run.return_value = subprocess.CompletedProcess([], 1)

        with self.assertRaises(BuildHygieneFailure):
            verify_and_clean_tauri_schemas()

        self.assertEqual(run.call_count, 1)

    @patch("tools.build_hygiene.subprocess.run")
    def test_清理后仍有未跟踪文件时失败(self, run: MagicMock) -> None:
        run.side_effect = [
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, stdout="?? unexpected.txt\n", stderr=""),
        ]

        with self.assertRaisesRegex(BuildHygieneFailure, "unexpected.txt"):
            verify_and_clean_tauri_schemas()


if __name__ == "__main__":
    unittest.main()
