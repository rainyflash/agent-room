from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from tools.release_promotion import PromotionFailure, advance, initialize, verify


class ReleasePromotionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.revision = "a" * 40

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_full_sequence_is_append_only_and_verifiable(self) -> None:
        current = self.root / "candidate.json"
        initialize("0.2.0", self.revision, current)
        for index, stage in enumerate(
            (
                "database-expanded",
                "compatible-server",
                "clients-published",
                "compatibility-observed",
                "legacy-contracted",
            ),
            start=1,
        ):
            following = self.root / f"{index}-{stage}.json"
            advance(
                current,
                following,
                stage,
                f"https://evidence.example/{stage}.json",
                f"{index:x}" * 64,
                1_800_000_000 + index,
            )
            current = following

        verify(current, "legacy-contracted", "0.2.0", self.revision)
        record = json.loads(current.read_text(encoding="utf-8"))
        self.assertEqual(len(record["history"]), 5)

    def test_stage_cannot_be_skipped(self) -> None:
        current = self.root / "candidate.json"
        initialize("0.2.0", self.revision, current)

        with self.assertRaisesRegex(PromotionFailure, "不能"):
            advance(
                current,
                self.root / "invalid.json",
                "compatible-server",
                "https://evidence.example/server.json",
                "b" * 64,
                1_800_000_000,
            )

    def test_tampered_history_is_rejected(self) -> None:
        current = self.root / "candidate.json"
        initialize("0.2.0", self.revision, current)
        document = json.loads(current.read_text(encoding="utf-8"))
        document["stage"] = "compatible-server"
        current.write_text(json.dumps(document), encoding="utf-8")

        with self.assertRaisesRegex(PromotionFailure, "历史"):
            verify(current, "compatible-server", "0.2.0", self.revision)


if __name__ == "__main__":
    unittest.main()
