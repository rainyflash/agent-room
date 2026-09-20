from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.release_ci import ReleaseCiFailure, build_metadata
from tools.release_recovery import main, validate_metadata, validate_source


class ReleaseRecoveryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.manifest = self.root / "Cargo.toml"
        self.manifest.write_text('[workspace.package]\nversion="0.2.0"\n', encoding="utf-8")
        self.args = argparse.Namespace(
            tag="v0.2.0", channel="testing", sequence=42, rollback_from="",
            revision="a" * 40, repository="example/agent-room",
            workspace_manifest=self.manifest, now_unix_seconds=1_800_000_000,
        )
        self.metadata = build_metadata(self.args)
        self.run = {
            "head_repository": {"full_name": self.args.repository},
            "repository": {"full_name": self.args.repository},
            "head_sha": self.args.revision, "head_branch": "main",
            "path": ".github/workflows/release-candidate.yml",
            "event": "workflow_dispatch", "status": "completed",
        }
        self.artifacts = [{"name": name, "expired": False} for name in (
            "release-metadata", "release-native-windows-x86_64", "release-native-darwin-aarch64",
            "release-image-control-plane", "release-image-identity", "release-image-web",
        )]

    def test_completed_main_build_can_be_recovered_with_original_identity(self) -> None:
        revision = validate_source(self.run, self.artifacts, self.args.repository, "full")
        recovered = validate_metadata(self.metadata, self.args, revision, 1_800_000_010)
        self.assertEqual(recovered, self.metadata)
        self.assertEqual(recovered["publishedAtUnixSeconds"], 1_800_000_000)

    def test_untrusted_or_unfinished_run_is_rejected(self) -> None:
        for key, value in (
            ("head_repository", {"full_name": "attacker/agent-room"}),
            ("repository", {"full_name": "attacker/agent-room"}),
            ("head_branch", "feature"), ("path", ".github/workflows/untrusted.yml"),
            ("event", "pull_request"), ("status", "in_progress"), ("head_sha", "main"),
        ):
            with self.subTest(key=key), self.assertRaises(ReleaseCiFailure):
                validate_source({**self.run, key: value}, self.artifacts,
                                self.args.repository, "full")

    def test_missing_duplicate_expired_or_wrong_profile_artifacts_are_rejected(self) -> None:
        variants = [self.artifacts[:-1], [*self.artifacts, self.artifacts[0]]]
        expired = copy.deepcopy(self.artifacts)
        expired[1]["expired"] = True
        variants.append(expired)
        variants.append([*self.artifacts, {"name": "release-candidate-v0.2.0", "expired": False}])
        for artifacts in variants:
            with self.subTest(artifacts=artifacts), self.assertRaises(ReleaseCiFailure):
                validate_source(self.run, artifacts, self.args.repository, "full")
        with self.assertRaises(ReleaseCiFailure):
            validate_source(self.run, self.artifacts, self.args.repository, "client")
        self.assertEqual(validate_source(self.run, self.artifacts[:3], self.args.repository,
                                        "client"), self.args.revision)

    def test_recovery_rejects_identity_changes_and_expiration_extension(self) -> None:
        for key, value in (
            ("revision", "b" * 40), ("sequence", 43), ("version", "0.3.0"),
            ("releaseBaseUrl", "https://attacker.example/release"),
            ("expiresAtUnixSeconds", 1_900_000_000), ("publishedAtUnixSeconds", True),
        ):
            with self.subTest(key=key), self.assertRaises(ReleaseCiFailure):
                validate_metadata({**self.metadata, key: value}, self.args,
                                  self.args.revision, 1_800_000_010)
        for now in (1_799_999_999, 1_800_604_800):
            with self.subTest(now=now), self.assertRaises(ReleaseCiFailure):
                validate_metadata(self.metadata, self.args, self.args.revision, now)

    def cli_args(self, run_id: str = "123") -> list[str]:
        return [
            "--run-id", run_id, "--repository", self.args.repository,
            "--tag", self.args.tag, "--channel", "testing", "--sequence", "42",
            "--profile", "full", "--workspace-manifest", str(self.manifest),
            "--metadata", str(self.root / "metadata.json"),
            "--promotion", str(self.root / "promotion.json"),
            "--github-output", str(self.root / "outputs.txt"),
        ]

    def test_recovery_checks_ancestry_and_emits_only_original_candidate_metadata(self) -> None:
        commands = []

        def execute(command):
            commands.append(command)
            if command[:2] == ("gh", "api"):
                value = ({"artifacts": self.artifacts, "total_count": len(self.artifacts)}
                         if "/artifacts?" in command[2] else self.run)
                return json.dumps(value)
            if command[:3] == ("gh", "run", "download"):
                directory = Path(command[command.index("--dir") + 1])
                (directory / "release-metadata.json").write_text(json.dumps(self.metadata),
                                                                encoding="utf-8")
            return ""

        with patch("tools.release_recovery.run_checked", side_effect=execute), \
                patch("tools.release_recovery.time.time", return_value=1_800_000_010):
            self.assertEqual(main(self.cli_args()), 0)
        self.assertIn(("git", "merge-base", "--is-ancestor", self.args.revision, "HEAD"), commands)
        self.assertEqual(json.loads((self.root / "metadata.json").read_text()), self.metadata)
        self.assertEqual(json.loads((self.root / "promotion.json").read_text())["stage"], "candidate")
        self.assertIn("artifact_run_id=123\n", (self.root / "outputs.txt").read_text())

    def test_invalid_run_id_never_reaches_external_commands(self) -> None:
        with patch("tools.release_recovery.run_checked") as execute:
            self.assertEqual(main(self.cli_args("123; echo unsafe")), 1)
            execute.assert_not_called()

    def test_non_ancestor_cannot_download_or_emit_candidate_metadata(self) -> None:
        def execute(command):
            if command[0] == "git":
                raise ReleaseCiFailure("source is not an ancestor")
            if command[:2] == ("gh", "api"):
                return json.dumps(
                    {"artifacts": self.artifacts, "total_count": len(self.artifacts)}
                    if "/artifacts?" in command[2] else self.run
                )
            self.fail("Recovery must stop before downloading unapproved source artifacts")

        with patch("tools.release_recovery.run_checked", side_effect=execute):
            self.assertEqual(main(self.cli_args()), 1)
        self.assertFalse((self.root / "metadata.json").exists())
        self.assertFalse((self.root / "outputs.txt").exists())


if __name__ == "__main__":
    unittest.main()
