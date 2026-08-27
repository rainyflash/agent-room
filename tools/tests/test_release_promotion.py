from __future__ import annotations

import json
import hashlib
from pathlib import Path
import tempfile
import unittest

from tools.release_promotion import (
    PromotionFailure,
    advance,
    initialize,
    verify,
    verify_evidence,
)


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

    def test_evidence_is_bound_to_release_asset_digest_and_candidate(self) -> None:
        current = self.root / "candidate.json"
        initialize("0.2.0", self.revision, current)
        evidence = self.write_evidence("database-expanded")
        promoted = self.root / "database-expanded.json"
        advance(
            current,
            promoted,
            "database-expanded",
            f"https://github.com/example/repo/releases/download/v0.2.0/{evidence.name}",
            self.digest(evidence),
            1_800_000_001,
        )

        verify_evidence(
            promoted,
            self.root,
            "https://github.com/example/repo/releases/download/v0.2.0/",
        )

    def test_evidence_rejects_tampered_asset_and_wrong_release(self) -> None:
        current = self.root / "candidate.json"
        initialize("0.2.0", self.revision, current)
        evidence = self.write_evidence("database-expanded")
        promoted = self.root / "database-expanded.json"
        advance(
            current,
            promoted,
            "database-expanded",
            f"https://github.com/example/repo/releases/download/v0.2.0/{evidence.name}",
            self.digest(evidence),
            1_800_000_001,
        )
        evidence.write_text("{}", encoding="utf-8")

        with self.assertRaisesRegex(PromotionFailure, "摘要"):
            verify_evidence(
                promoted,
                self.root,
                "https://github.com/example/repo/releases/download/v0.2.0/",
            )
        with self.assertRaisesRegex(PromotionFailure, "不属于"):
            verify_evidence(
                promoted,
                self.root,
                "https://github.com/example/repo/releases/download/v0.3.0/",
            )

    def test_evidence_rejects_failed_check(self) -> None:
        current = self.root / "candidate.json"
        initialize("0.2.0", self.revision, current)
        evidence = self.write_evidence("database-expanded", passed=False)
        promoted = self.root / "database-expanded.json"
        advance(
            current,
            promoted,
            "database-expanded",
            f"https://github.com/example/repo/releases/download/v0.2.0/{evidence.name}",
            self.digest(evidence),
            1_800_000_001,
        )

        with self.assertRaisesRegex(PromotionFailure, "passed"):
            verify_evidence(
                promoted,
                self.root,
                "https://github.com/example/repo/releases/download/v0.2.0/",
            )

    def write_evidence(self, stage: str, *, passed: bool = True) -> Path:
        path = self.root / f"{stage}-evidence.json"
        path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "kind": "agent-room.release-deployment-evidence",
                    "stage": stage,
                    "version": "0.2.0",
                    "revision": self.revision,
                    "capturedAtUnixSeconds": 1_800_000_000,
                    "checks": [
                        {"name": "real-probe", "passed": passed, "detail": "probe result"}
                    ],
                    "result": "passed",
                }
            ),
            encoding="utf-8",
        )
        return path

    @staticmethod
    def digest(path: Path) -> str:
        return hashlib.sha256(path.read_bytes()).hexdigest()


if __name__ == "__main__":
    unittest.main()
