from __future__ import annotations

import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock
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

    def reuse_device(self, captured_ago: int = 3600, **report_fields):
        report = self.reports["first-device"]
        report["checks"] = {key: True for key in acceptance.REUSED_DEVICE_CHECKS}
        report["deviceMode"] = "reused"
        report["freshAuthorization"] = {"version": "0.1.9", "revision": "c" * 40,
                                        "capturedAtUnixSeconds": self.now - captured_ago}
        report.update(report_fields)
        self.reindex()

    def test_long_lived_device_is_accepted_only_while_the_policy_allows(self):
        self.reuse_device()
        acceptance.verify(self.root, self.version, self.revision, now=self.now, changes=lambda base, head: [])
        with self.assertRaisesRegex(release.ReleaseFailure, "登录相关代码"):
            acceptance.verify(self.root, self.version, self.revision, now=self.now,
                              changes=lambda base, head: ["crates/identity-adapter/src/device_grant.rs"])
        self.reuse_device(captured_ago=acceptance.FRESH_AUTHORIZATION_MAX_AGE_SECONDS + 1)
        with self.assertRaisesRegex(release.ReleaseFailure, "30 天"):
            acceptance.verify(self.root, self.version, self.revision, now=self.now, changes=lambda base, head: [])

    def test_long_lived_device_cannot_claim_a_new_authorization_or_an_unknown_mode(self):
        self.reuse_device()
        self.reports["first-device"]["checks"]["authorizationCompleted"] = True
        self.reindex()
        with self.assertRaises(release.ReleaseFailure):
            acceptance.verify(self.root, self.version, self.revision, now=self.now, changes=lambda base, head: [])
        self.reuse_device(deviceMode="borrowed")
        with self.assertRaisesRegex(release.ReleaseFailure, "设备模式"):
            acceptance.verify(self.root, self.version, self.revision, now=self.now, changes=lambda base, head: [])
        self.reuse_device(freshAuthorization={"version": "0.1.9", "revision": "not-a-revision",
                                              "capturedAtUnixSeconds": self.now})
        with self.assertRaises(release.ReleaseFailure):
            acceptance.verify(self.root, self.version, self.revision, now=self.now, changes=lambda base, head: [])

    MOVE = {"from": "old.example", "to": "new.example"}

    def migrate_upgrade(self, **report_fields):
        report = self.reports["upgrade"]
        report["checks"] = {key: True for key in acceptance.MIGRATED_UPGRADE_CHECKS}
        report["upgradeMode"] = "server-migration"
        report["serverMigration"] = dict(self.MOVE)
        report.update(report_fields)
        self.reindex()

    def test_server_migration_upgrade_is_accepted_only_for_the_listed_move(self):
        self.migrate_upgrade()
        with self.assertRaisesRegex(release.ReleaseFailure, "登记"):
            self.verify()
        with mock.patch.dict(acceptance.SERVER_MIGRATIONS, {self.version: self.MOVE}):
            self.verify()
            self.migrate_upgrade(serverMigration={"from": "old.example", "to": "elsewhere.example"})
            with self.assertRaisesRegex(release.ReleaseFailure, "登记"):
                self.verify()

    def test_server_migration_upgrade_cannot_claim_kept_state_or_skip_checks(self):
        with mock.patch.dict(acceptance.SERVER_MIGRATIONS, {self.version: self.MOVE}):
            self.migrate_upgrade()
            self.reports["upgrade"]["checks"]["identityPreserved"] = True
            self.reindex()
            with self.assertRaisesRegex(release.ReleaseFailure, "不能声称"):
                self.verify()
            self.migrate_upgrade()
            del self.reports["upgrade"]["checks"]["signedInToNewServer"]
            self.reindex()
            with self.assertRaises(release.ReleaseFailure):
                self.verify()
            self.migrate_upgrade(upgradeMode="partial")
            with self.assertRaisesRegex(release.ReleaseFailure, "升级模式"):
                self.verify()

    def test_login_path_changes_are_read_from_history(self):
        head = acceptance.login_changes("HEAD", "HEAD")
        self.assertEqual(head, [])
        with self.assertRaises(release.ReleaseFailure):
            acceptance.login_changes("0" * 40, "HEAD")

    def test_assembly_does_not_write_partial_invalid_evidence(self):
        (self.root / "release-usability-acceptance.json").unlink()
        report = self.reports["upgrade"]
        report["evidence"][0]["path"] = "../outside.txt"
        self.write("release-usability-upgrade.json", report)
        with self.assertRaises(release.ReleaseFailure):
            acceptance.assemble(self.root, [self.root / f"release-usability-{scenario}.json" for scenario in self.reports])
        self.assertFalse((self.root / "release-usability-acceptance.json").exists())


if __name__ == "__main__": unittest.main()
