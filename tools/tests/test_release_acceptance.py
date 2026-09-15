from __future__ import annotations

import json
from pathlib import Path
import tempfile
import time
import unittest
from uuid import UUID

from tools import release_acceptance as acceptance, release


class UsabilityAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.version, self.revision, self.now = "0.2.0", "a" * 40, int(time.time())
        (self.root / "release.signed.json").write_text("unit-test-only", encoding="utf-8")
        self.write("release-metadata.json", {"version": self.version, "revision": self.revision, "publishedAtUnixSeconds": self.now - 5})
        digest = release.sha256_file(self.root / "release.signed.json")
        self.reports = {}
        for index, (scenario, checks) in enumerate(acceptance.SCENARIOS.items()):
            evidence = self.root / f"usability-evidence-{scenario}.txt"
            evidence.write_text("synthetic test data, never publication evidence", encoding="utf-8")
            report = {"schemaVersion": 1, "scenario": scenario, "version": self.version, "revision": self.revision,
                      "signedManifestSha256": digest, "result": "passed", "fixture": False,
                      "capturedAtUnixSeconds": self.now, "checks": {key: True for key in checks},
                      "evidence": [{"path": evidence.name, "sha256": release.sha256_file(evidence), "redacted": True}]}
            if index == 2:
                report["replyReceipts"] = [{"eventId": f"$reply{i}", "submissionId": str(UUID(f"0198b601-77a1-7bb8-83eb-{i:012d}"))} for i in (1, 2)]
            self.reports[scenario] = report
        self.reindex()

    def write(self, name, value):
        (self.root / name).write_text(json.dumps(value), encoding="utf-8")

    def reindex(self):
        references = {}
        for scenario, report in self.reports.items():
            name = f"release-usability-{scenario}.json"
            self.write(name, report)
            references[scenario] = {"path": name, "sha256": release.sha256_file(self.root / name)}
        self.write("release-usability-acceptance.json", {"schemaVersion": 1, "version": self.version, "revision": self.revision,
                    "signedManifestSha256": release.sha256_file(self.root / "release.signed.json"), "reports": references})

    def verify(self): acceptance.verify(self.root, self.version, self.revision, now=self.now)

    def test_complete_same_candidate_evidence_and_idempotent_assembly(self):
        self.verify()
        # Assembly has canonical formatting and must not overwrite an existing index.
        (self.root / "release-usability-acceptance.json").unlink()
        paths = [self.root / f"release-usability-{scenario}.json" for scenario in self.reports]
        acceptance.assemble(self.root, paths)
        acceptance.assemble(self.root, paths)
        self.verify()

    def test_missing_failed_or_fixture_run_never_passes(self):
        for field, value in (("fixture", True), ("result", "failed"), ("revision", "b" * 40), ("signedManifestSha256", "0" * 64), ("capturedAtUnixSeconds", self.now - 100)):
            with self.subTest(field=field):
                report = self.reports["first-device"]
                original = report[field]
                report[field] = value
                self.reindex()
                with self.assertRaises(release.ReleaseFailure): self.verify()
                report[field] = original
        self.reports["upgrade"]["checks"]["identityPreserved"] = False
        self.reindex()
        with self.assertRaises(release.ReleaseFailure): self.verify()

    def test_missing_tampered_or_duplicate_reply_evidence_is_rejected(self):
        report = self.reports["continuous-reception"]
        report["replyReceipts"][1] = report["replyReceipts"][0]
        self.reindex()
        with self.assertRaises(release.ReleaseFailure): self.verify()
        (self.root / "usability-evidence-first-device.txt").write_text("tampered", encoding="utf-8")
        with self.assertRaises(release.ReleaseFailure): self.verify()

    def test_assembly_does_not_write_partial_invalid_evidence(self):
        (self.root / "release-usability-acceptance.json").unlink()
        report = self.reports["upgrade"]
        report["evidence"][0]["path"] = "../outside.txt"
        self.write("release-usability-upgrade.json", report)
        with self.assertRaises(release.ReleaseFailure):
            acceptance.assemble(self.root, [self.root / f"release-usability-{scenario}.json" for scenario in self.reports])
        self.assertFalse((self.root / "release-usability-acceptance.json").exists())


if __name__ == "__main__": unittest.main()
