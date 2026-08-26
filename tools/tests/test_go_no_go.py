from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from tools.go_no_go import (
    GoNoGoFailure,
    REQUIRED_GATES,
    render_record,
    validate_decision,
)


class GoNoGoTests(unittest.TestCase):
    def test_no_go_requires_complete_matrix_and_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            decision = fixture_decision(root)
            validate_decision(decision, root)
            record = render_record(decision, root, root / "record.md")
            self.assertIn("结论：NO-GO", record)
            self.assertIn("需求 1–15 验收矩阵", record)

    def test_go_rejects_open_blockers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            decision = fixture_decision(root)
            decision["decision"] = "go"
            decision["publicBetaEnabled"] = True
            with self.assertRaises(GoNoGoFailure):
                validate_decision(decision, root)

    def test_missing_requirement_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            decision = fixture_decision(root)
            requirements = decision["requirements"]
            assert isinstance(requirements, list)
            requirements.pop()
            with self.assertRaises(GoNoGoFailure):
                validate_decision(decision, root)

    def test_evidence_must_remain_inside_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            decision = fixture_decision(root)
            requirements = decision["requirements"]
            assert isinstance(requirements, list)
            first = requirements[0]
            assert isinstance(first, dict)
            first["evidence"] = ["../outside.md"]
            with self.assertRaises(GoNoGoFailure):
                validate_decision(decision, root)


def fixture_decision(root: Path) -> dict[str, object]:
    evidence = root / "evidence.md"
    evidence.write_text("# Evidence\n", encoding="utf-8")
    requirements = [
        {
            "id": identifier,
            "name": f"需求 {identifier}",
            "status": "blocked" if identifier == 15 else "pass",
            "evidence": ["evidence.md"],
            "blockerIds": ["GNG-001"] if identifier == 15 else [],
        }
        for identifier in range(1, 16)
    ]
    gates = [
        {
            "id": identifier,
            "name": identifier,
            "status": "blocked" if identifier == "release" else "pass",
            "evidence": ["evidence.md"],
            "blockerIds": ["GNG-001"] if identifier == "release" else [],
        }
        for identifier in sorted(REQUIRED_GATES)
    ]
    return {
        "schemaVersion": 1,
        "target": "public-beta-v0.1.0",
        "recordedAt": "2026-08-26",
        "baselineRevision": "a" * 40,
        "decision": "no-go",
        "publicBetaEnabled": False,
        "requirements": requirements,
        "gates": gates,
        "blockers": [
            {
                "id": "GNG-001",
                "severity": "blocking",
                "status": "open",
                "title": "发行证据缺失",
                "owner": "维护者",
                "exitCondition": "真实发行门通过",
                "evidence": ["evidence.md"],
            }
        ],
        "publication": {
            "releaseNotes": "evidence.md",
            "knownLimitations": "evidence.md",
            "dataPolicy": "evidence.md",
            "securityPolicy": "evidence.md",
        },
    }


if __name__ == "__main__":
    unittest.main()
