from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from tools.database import (
    postgres_correctness_test_command,
    postgres_correctness_test_targets,
    postgres_coverage_test_command,
)


class PostgreSqlCoverageCommandTests(unittest.TestCase):
    def test_coverage_excludes_capacity_budget(self) -> None:
        with TemporaryDirectory() as directory:
            tests_directory = Path(directory)
            (tests_directory / "authentication.rs").touch()
            (tests_directory / "capacity.rs").touch()
            (tests_directory / "devices.rs").touch()

            targets = postgres_correctness_test_targets(tests_directory)
            correctness_command = postgres_correctness_test_command(tests_directory)
            coverage_command = postgres_coverage_test_command(tests_directory)

        self.assertEqual(targets, ("authentication", "devices"))
        self.assertNotIn("capacity", correctness_command)
        self.assertNotIn("capacity", coverage_command)
        self.assertEqual(correctness_command.count("--test"), 2)
        self.assertEqual(coverage_command.count("--test"), 2)

    def test_missing_capacity_target_fails_loudly(self) -> None:
        with TemporaryDirectory() as directory:
            tests_directory = Path(directory)
            (tests_directory / "authentication.rs").touch()

            with self.assertRaisesRegex(RuntimeError, "性能测试目标缺失"):
                postgres_correctness_test_targets(tests_directory)


if __name__ == "__main__":
    unittest.main()
