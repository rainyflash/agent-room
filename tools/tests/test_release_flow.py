from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools import release_flow as flow, release, release_deploy


class ReleaseFlowTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.state = {"schemaVersion": 1, "repository": "rainyflash/agent-room", "revision": "a" * 40,
                      "version": "0.2.0", "tag": "v0.2.0", "sequence": 42, "profile": "client",
                      "installedVersion": "0.1.0", "highestSequence": 41, "runs": {}}
        self.github = Mock(spec=flow.GitHub)
        self.github.repository = self.state["repository"]
        self.github.main_revision.return_value = self.state["revision"]
        self.subject = flow.ReleaseFlow(self.root / "state.json", self.state, self.github)

    def run_result(self, **options):
        return {"headSha": self.state["revision"], "headBranch": "main", "status": "completed",
                "conclusion": "success", "jobs": [{"name": name, "conclusion": "success"} for name in flow.CI_JOBS], **options}

    def test_ambiguous_dispatch_is_persisted_and_never_dispatched_twice(self):
        self.github.json.return_value = []
        self.github.call.side_effect = RuntimeError("network response lost")
        with self.assertRaises(RuntimeError):
            self.subject.workflow("ci.yml", {"suite": "all"})
        saved = json.loads(self.subject.path.read_text(encoding="utf-8"))
        self.assertTrue(saved["runs"]["ci.yml"]["dispatched"])
        self.github.call.side_effect = None
        resumed = flow.ReleaseFlow(self.subject.path, saved, self.github)
        with self.assertRaises(flow.Waiting) as waiting:
            resumed.workflow("ci.yml", {"suite": "all"})
        self.assertFalse(waiting.exception.pollable)
        self.github.call.assert_called_once()
        operation = saved["runs"]["ci.yml"]["operation"]
        self.github.json.side_effect = [[{"databaseId": 12, "displayTitle": f"release-flow:{operation}", "headSha": self.state["revision"]}], self.run_result()]
        resumed.workflow("ci.yml", {"suite": "all"}, required=flow.CI_JOBS)
        self.assertEqual(resumed.state["runs"]["ci.yml"]["id"], 12)
        self.github.call.assert_called_once()

    def test_publication_runs_from_a_protected_branch_pinned_to_the_candidate(self):
        self.github.json.return_value = []
        self.github.on_main.return_value = True
        self.github.branch_revision.return_value = None
        self.github.branch_protected.return_value = True
        with self.assertRaises(flow.Waiting) as waiting:
            self.subject.workflow("release-publish.yml", {"tag": "v0.2.0"})
        self.assertTrue(waiting.exception.pollable)
        # main may have moved on; publication no longer depends on its head.
        self.github.main_revision.assert_not_called()
        self.github.create_branch.assert_called_once_with("release/v0.2.0", self.state["revision"])
        arguments = self.github.call.call_args.args
        self.assertEqual(arguments[:5], ("workflow", "run", "release-publish.yml", "--ref", "release/v0.2.0"))
        self.assertIn(f"expected_revision={self.state['revision']}", arguments)
        self.assertEqual(self.github.json.call_args.args[4:6], ("--branch", "release/v0.2.0"))
        operation = self.state["runs"]["release-publish.yml"]["operation"]
        found = [{"databaseId": 14, "displayTitle": f"release-flow:{operation}", "headSha": self.state["revision"]}]
        self.github.json.side_effect = [found, self.run_result(headBranch="main")]
        with self.assertRaisesRegex(RuntimeError, "release/v0.2.0"):
            self.subject.workflow("release-publish.yml", {"tag": "v0.2.0"})
        self.github.json.side_effect = [self.run_result(headBranch="release/v0.2.0")]
        self.subject.workflow("release-publish.yml", {"tag": "v0.2.0"})
        self.assertEqual(self.state["runs"]["release-publish.yml"]["id"], 14)
        self.github.call.assert_called_once()

    def test_publication_never_moves_or_trusts_an_unsuitable_release_branch(self):
        self.github.json.return_value = []
        cases = [
            ({"on_main": False, "branch_revision": None, "branch_protected": True}, "不在受保护的 main"),
            ({"on_main": True, "branch_revision": "b" * 40, "branch_protected": True}, "不会移动"),
            ({"on_main": True, "branch_revision": self.state["revision"], "branch_protected": False}, "未受保护"),
        ]
        for answers, message in cases:
            with self.subTest(message=message):
                for name, value in answers.items():
                    getattr(self.github, name).return_value = value
                with self.assertRaisesRegex(RuntimeError, message):
                    self.subject.workflow("release-publish.yml", {"tag": "v0.2.0"})
                self.assertFalse(self.state["runs"]["release-publish.yml"]["dispatched"])
        self.github.create_branch.assert_not_called()
        self.github.call.assert_not_called()

    def test_only_incomplete_successfully_dispatched_runs_are_pollable(self):
        with self.assertRaises(flow.Waiting) as pending:
            flow.completed_run(self.run_result(status="in_progress"), self.state["revision"])
        self.assertTrue(pending.exception.pollable)
        with self.assertRaises(flow.Waiting) as failed:
            flow.completed_run(self.run_result(conclusion="failure"), self.state["revision"])
        self.assertFalse(failed.exception.pollable)
        for result in [self.run_result(headBranch="feature"), self.run_result(headSha="b" * 40), self.run_result(jobs=[])]:
            with self.assertRaises(RuntimeError):
                flow.completed_run(result, self.state["revision"], flow.CI_JOBS)

    def test_watch_retries_only_incomplete_stages_and_new_process_rechecks_gates(self):
        calls = []
        blocked = True
        def acceptance():
            calls.append("acceptance")
            if blocked: raise flow.Waiting("real host evidence missing")
        self.subject.workflow = lambda name, *_args, **_kwargs: calls.append(name)
        self.subject.download = lambda: calls.append("download")
        self.subject.verify_candidate = lambda: calls.append("verify")
        self.subject.compatible_server = lambda: calls.append("server")
        self.subject.acceptance = acceptance
        self.subject.verify_publication = lambda: calls.append("channel")
        with patch.object(flow, "command", side_effect=lambda command: self.state["revision"] if command[1] == "rev-parse" else ""):
            with self.assertRaises(flow.Waiting): self.subject.advance()
            blocked = False
            self.subject.advance()
        self.assertEqual(calls.count("verify"), 1)
        self.assertEqual(calls.count("ci.yml"), 1)
        self.assertEqual(calls.count("acceptance"), 2)
        self.assertLess(calls.index("server"), calls.index("release-publish.yml"))
        self.assertEqual(calls[-1], "channel")
        fresh = flow.ReleaseFlow(self.subject.path, self.state, self.github)
        self.assertEqual(fresh.completed, set())

    def test_changed_candidate_is_not_verified_or_reused(self):
        self.subject.candidate.mkdir()
        (self.subject.candidate / "release.signed.json").write_text("replacement", encoding="utf-8")
        key = self.root / "key.json"
        key.write_text("independent trust", encoding="utf-8")
        self.state.update(trustedPublicKey=str(key), trustedPublicKeySha256=release.sha256_file(key), signedManifestSha256="0" * 64)
        metadata = {key: self.state[key] for key in ("version", "tag", "revision", "sequence")}
        metadata["channel"] = "testing"
        flow.atomic_json(self.subject.candidate / "release-metadata.json", metadata)
        with patch.object(release, "verify") as verify:
            with self.assertRaisesRegex(RuntimeError, "候选已被替换"):
                self.subject.verify_candidate()
            verify.assert_not_called()

    def test_immutable_upload_reconciles_equal_assets_and_refuses_replacements(self):
        asset = self.root / "evidence.json"
        asset.write_text("original", encoding="utf-8")
        self.github.json.return_value = {"assets": [{"name": asset.name, "digest": f"sha256:{release.sha256_file(asset)}"}]}
        self.subject.upload([asset])
        self.github.call.assert_not_called()
        check = self.root / "remote-assets"
        check.mkdir()
        (check / asset.name).write_text("other", encoding="utf-8")
        self.github.json.return_value = {"assets": [{"name": asset.name}]}
        with self.assertRaisesRegex(RuntimeError, "不同内容"):
            self.subject.upload([asset])
        self.assertEqual(self.github.call.call_args.args[0:2], ("release", "download"))

    def test_deployment_overlay_only_changes_application_images(self):
        images = {name: f"ghcr.io/rainyflash/agent-room/{name}@sha256:{'a' * 64}" for name in ("control-plane", "identity", "web")}
        overlay = release_deploy.image_overlay(images)
        self.assertEqual(set(overlay["services"]), {"control-plane", "identity", "gateway", "migrate", "object-store-init"})
        self.assertEqual(overlay["services"]["gateway"]["image"], images["web"])
        with self.assertRaises(RuntimeError): release_deploy.image_overlay({**images, "web": "ghcr.io/rainyflash/web:latest"})

    def test_failed_step_keeps_checkpoint_without_claiming_completion(self):
        operation = Mock(side_effect=RuntimeError("deployment failed"))
        with self.assertRaises(RuntimeError): self.subject.step("server", operation)
        self.assertEqual(json.loads(self.subject.path.read_text(encoding="utf-8"))["stage"], "server")
        self.assertNotIn("server", self.subject.completed)
        operation.side_effect = None
        self.subject.step("server", operation)
        self.subject.step("server", operation)
        self.assertEqual(operation.call_count, 2)

    def recovery_source(self):
        return {"head_sha": self.state["revision"], "head_branch": "main", "event": "workflow_dispatch", "status": "completed", "path": ".github/workflows/release-candidate.yml", "repository": {"full_name": self.github.repository}, "head_repository": {"full_name": self.github.repository}}

    def test_failed_upload_recovers_archive_before_any_release_mutation(self):
        artifact = {"name": "release-candidate-v0.2.0", "expired": False}
        with patch.object(flow, "command", side_effect=[json.dumps(self.recovery_source()), json.dumps({"total_count": 1, "artifacts": [artifact]})]), patch.object(self.subject, "verify_candidate", side_effect=RuntimeError("bad signature")):
            with self.assertRaisesRegex(RuntimeError, "bad signature"):
                self.subject.recover_candidate(12)
        self.github.call.assert_called_once()
        self.assertEqual(self.github.call.call_args.args[:2], ("run", "download"))
        self.assertFalse(self.state.get("downloaded", False))

    def test_missing_signed_archive_never_starts_another_build(self):
        with patch.object(flow, "command", side_effect=[json.dumps(self.recovery_source()), json.dumps({"total_count": 0, "artifacts": []})]):
            with self.assertRaises(flow.Waiting): self.subject.recover_candidate(12)
        self.github.call.assert_not_called()

    def test_recovered_candidate_resumes_after_publication_without_needing_archive_or_draft(self):
        self.state.update(downloaded=True, recoveredCandidateRun=12)
        self.state["runs"]["release-candidate.yml"] = {"operation": "operation", "dispatched": True, "id": 12}
        self.github.json.return_value = self.run_result(conclusion="failure")
        with patch.object(self.subject, "verify_candidate") as verify, patch.object(self.subject, "recover_candidate") as recover:
            self.subject.workflow("release-candidate.yml", {})
            verify.assert_called_once()
            recover.assert_not_called()
        self.github.call.assert_not_called()

    def test_candidate_recovery_is_checkpointed_only_after_verified_upload(self):
        artifact = {"name": "release-candidate-v0.2.0", "expired": False}
        responses = [json.dumps(self.recovery_source()), json.dumps({"total_count": 1, "artifacts": [artifact]}), "[[]]"]
        with patch.object(flow, "command", side_effect=responses), patch.object(self.subject, "verify_candidate"), patch.object(self.subject, "upload", side_effect=RuntimeError("upload uncertain")):
            with self.assertRaisesRegex(RuntimeError, "upload uncertain"):
                self.subject.recover_candidate(12)
        self.assertNotIn("recoveredCandidateRun", self.state)
        with patch.object(flow, "command", side_effect=responses), patch.object(self.subject, "verify_candidate"), patch.object(self.subject, "upload"):
            self.subject.recover_candidate(12)
        saved = json.loads(self.subject.path.read_text(encoding="utf-8"))
        self.assertEqual(saved["recoveredCandidateRun"], 12)
        self.assertTrue(saved["downloaded"])

    def test_recovery_rejects_missing_repository_identity_before_downloading(self):
        for value in (None, [], "rainyflash/agent-room"):
            source = {**self.recovery_source(), "head_repository": value}
            with self.subTest(value=value), patch.object(flow, "command", return_value=json.dumps(source)):
                with self.assertRaisesRegex(RuntimeError, "来源"):
                    self.subject.recover_candidate(12)
        self.github.call.assert_not_called()


if __name__ == "__main__": unittest.main()
