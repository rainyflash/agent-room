from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.release_promotion import (
    PromotionFailure,
    advance,
    create_evidence,
    initialize,
    parse_evidence_check,
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

    def test_resume_retains_original_bytes_and_rejects_changed_evidence(self) -> None:
        current, following = self.root / "candidate.json", self.root / "next.json"
        initialize("0.2.0", self.revision, current)
        arguments = (current, following, "database-expanded", "https://evidence.example/database.json", "b" * 64)
        advance(*arguments, 1_800_000_000)
        original = following.read_bytes()
        advance(*arguments, 1_800_000_060, resume=True)
        self.assertEqual(following.read_bytes(), original)
        with self.assertRaisesRegex(PromotionFailure, "不能覆盖"):
            advance(*arguments[:-1], "c" * 64, 1_800_000_060, resume=True)
        with self.assertRaisesRegex(PromotionFailure, "晚于"):
            advance(*arguments, 1_799_999_999, resume=True)
        self.assertEqual(following.read_bytes(), original)

    def test_resume_does_not_accept_a_record_for_a_different_release(self) -> None:
        current, following = self.root / "candidate.json", self.root / "next.json"
        other = self.root / "other.json"
        initialize("0.2.0", self.revision, current)
        initialize("0.3.0", self.revision, other)
        advance(other, following, "database-expanded", "https://evidence.example/database.json", "b" * 64, 1_800_000_000)
        original = following.read_bytes()
        with self.assertRaisesRegex(PromotionFailure, "不一致"):
            advance(current, following, "database-expanded", "https://evidence.example/database.json", "b" * 64, 1_800_000_060, resume=True)
        self.assertEqual(following.read_bytes(), original)

    def test_create_evidence_writes_valid_checked_document(self) -> None:
        output = self.root / "database-expanded-evidence.json"

        create_evidence(
            "0.2.0",
            self.revision,
            "database-expanded",
            (("backup-verified", "最新生产备份及摘要验证通过。"),),
            1_800_000_000,
            output,
        )

        document = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(document["kind"], "agent-room.release-deployment-evidence")
        self.assertEqual(document["stage"], "database-expanded")
        self.assertEqual(
            document["checks"],
            [
                {
                    "name": "backup-verified",
                    "passed": True,
                    "detail": "最新生产备份及摘要验证通过。",
                }
            ],
        )

    def test_create_evidence_rejects_empty_checks_and_overwrite(self) -> None:
        output = self.root / "compatible-server-evidence.json"
        with self.assertRaises(PromotionFailure):
            create_evidence(
                "0.2.0",
                self.revision,
                "compatible-server",
                (),
                1_800_000_000,
                output,
            )

        create_evidence(
            "0.2.0",
            self.revision,
            "compatible-server",
            (("health", "服务健康。"),),
            1_800_000_000,
            output,
        )
        with self.assertRaises(PromotionFailure):
            create_evidence(
                "0.2.0",
                self.revision,
                "compatible-server",
                (("health", "服务健康。"),),
                1_800_000_000,
                output,
            )

    def test_parse_evidence_check_requires_name_and_detail(self) -> None:
        self.assertEqual(parse_evidence_check("health=服务健康"), ("health", "服务健康"))
        with self.assertRaises(argparse.ArgumentTypeError):
            parse_evidence_check("health=")

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
