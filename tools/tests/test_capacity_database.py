from __future__ import annotations

import unittest

from tools.capacity_database import (
    DatabaseCapacityFailure,
    capacity_test_command,
    parse_observation,
)


class DatabaseCapacityObservationTests(unittest.TestCase):
    def test_parser_accepts_one_machine_record(self) -> None:
        observation = parse_observation(
            'test output\ntest capacity ... CAPACITY_OBSERVATION={"agents":10000,"passed":true}\n'
        )
        self.assertEqual(observation["agents"], 10_000)
        self.assertTrue(observation["passed"])

    def test_parser_rejects_missing_or_duplicate_records(self) -> None:
        with self.assertRaisesRegex(DatabaseCapacityFailure, "唯一观察"):
            parse_observation("no observation")
        with self.assertRaisesRegex(DatabaseCapacityFailure, "唯一观察"):
            parse_observation(
                "CAPACITY_OBSERVATION={}\nCAPACITY_OBSERVATION={}\n"
            )

    def test_capacity_budget_uses_release_binary(self) -> None:
        command = capacity_test_command()
        self.assertEqual(command[:3], ["cargo", "test", "--release"])
        self.assertIn("capacity", command)


if __name__ == "__main__":
    unittest.main()
