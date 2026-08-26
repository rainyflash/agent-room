from __future__ import annotations

from pathlib import Path
import subprocess
import unittest
from unittest.mock import call, patch

from tools.source_revision import (
    SourceRevisionFailure,
    clean_git_revision,
    require_clean_git_revision,
)


class CleanGitRevisionTests(unittest.TestCase):
    @patch("tools.source_revision.subprocess.run")
    def test_returns_verified_revision_only_for_clean_worktree(self, run) -> None:
        revision = "a" * 40
        run.side_effect = [
            subprocess.CompletedProcess([], 0, stdout="", stderr=""),
            subprocess.CompletedProcess([], 0, stdout=f"{revision}\n", stderr=""),
        ]

        self.assertEqual(clean_git_revision(Path("repository")), revision)
        self.assertEqual(
            run.call_args_list,
            [
                call(
                    [
                        "git",
                        "status",
                        "--porcelain=v1",
                        "--untracked-files=normal",
                        "--ignore-submodules=none",
                    ],
                    cwd=Path("repository"),
                    check=False,
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                ),
                call(
                    ["git", "rev-parse", "--verify", "HEAD^{commit}"],
                    cwd=Path("repository"),
                    check=False,
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                ),
            ],
        )

    @patch("tools.source_revision.subprocess.run")
    def test_rejects_dirty_worktree_before_reading_revision(self, run) -> None:
        run.return_value = subprocess.CompletedProcess(
            [], 0, stdout=" M tools/capacity.py\n?? untracked.py\n", stderr=""
        )

        with self.assertRaisesRegex(SourceRevisionFailure, "干净 Git 工作树"):
            clean_git_revision(Path("repository"))

        run.assert_called_once()

    @patch("tools.source_revision.subprocess.run")
    def test_rejects_invalid_or_unreadable_revision(self, run) -> None:
        run.side_effect = [
            subprocess.CompletedProcess([], 0, stdout="", stderr=""),
            subprocess.CompletedProcess([], 0, stdout="not-a-revision\n", stderr=""),
        ]

        with self.assertRaisesRegex(SourceRevisionFailure, "完整 Git 提交"):
            clean_git_revision(Path("repository"))

    @patch("tools.source_revision.clean_git_revision", return_value="b" * 40)
    def test_rejects_revision_changed_during_acceptance(self, clean_revision) -> None:
        with self.assertRaisesRegex(SourceRevisionFailure, f"开始 {'a' * 40}"):
            require_clean_git_revision(Path("repository"), "a" * 40)

        clean_revision.assert_called_once_with(Path("repository"))


if __name__ == "__main__":
    unittest.main()
