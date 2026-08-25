from __future__ import annotations

import unittest

from tools.object_store import parse_capacity_observation


class ObjectStoreCapacityObservationTests(unittest.TestCase):
    def test_parser_accepts_test_harness_prefix(self) -> None:
        observation = parse_capacity_observation(
            'test capacity ... CAPACITY_CONTENT_OBSERVATION={"attachmentBytes":26214400}\n'
        )
        self.assertEqual(observation["attachmentBytes"], 25 * 1_024 * 1_024)

    def test_parser_rejects_ambiguous_output(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "唯一观察"):
            parse_capacity_observation("没有容量观察")


if __name__ == "__main__":
    unittest.main()
