from __future__ import annotations

import argparse
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, call

from tools import release_deploy


REVISION = "a" * 40
SERVICES = {name: {"containerId": name, "imageId": name, "image": name, "health": "healthy"}
            for name in ("postgres", "synapse", "object-store", "identity", "control-plane", "gateway")}


class DeploymentPrepareTests(unittest.TestCase):
    def deployment(self, work: Path) -> tuple[release_deploy.Deployment, Mock, dict[str, str]]:
        # Deployment() refuses to run outside Linux; build one around mocks instead.
        deployment = object.__new__(release_deploy.Deployment)
        deployment.args = argparse.Namespace(revision=REVISION, manifest_sha256="b" * 64)
        deployment.images = {name: f"ghcr.io/rainyflash/agent-room/{name}@sha256:{'c' * 64}"
                             for name in ("control-plane", "identity", "web")}
        deployment.overlay = release_deploy.image_overlay(deployment.images)
        deployment.work = work
        deployment.checkpoint = work / "deployment.json"
        deployment.state = {}
        digest = {"value": "before-render"}
        steps = Mock()
        steps.backup.return_value = Mock(backup_id="backup-1")
        # The candidate's renderer changes the configuration, as Alpha 44's realm theme keys did.
        steps.prepare.side_effect = lambda **_: digest.update(value="after-render")
        deployment.runtime = steps
        deployment.configuration_digest = lambda: digest["value"]
        deployment.services = Mock(return_value=SERVICES)
        deployment.compose = Mock()
        deployment.run = Mock(side_effect=lambda *arguments: REVISION if arguments[:2] == ("git", "rev-parse") else "")
        return deployment, steps, digest

    def test_renders_with_the_candidate_after_the_backup_and_records_that_configuration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            deployment, steps, digest = self.deployment(Path(directory))
            deployment.prepare()

            order = [name for name, *_ in steps.mock_calls if name in ("backup", "verify_backup", "prepare")]
            self.assertEqual(order[:3], ["backup", "verify_backup", "prepare"])
            steps.backup.assert_called_once_with(preserve_configuration=True)
            self.assertIn(call.prepare(generate_signing_key=True), steps.mock_calls)
            self.assertEqual(deployment.state["configurationSha256"], "after-render")
            self.assertTrue(deployment.state["renderedConfigurationChanged"])
            # A later scheduled backup renders the same configuration, so the deployment continues.
            deployment.baseline()
            digest["value"] = "changed-by-someone-else"
            with self.assertRaisesRegex(RuntimeError, "配置或密钥变化"):
                deployment.baseline()

    def test_resumed_deployment_does_not_render_or_back_up_again(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            deployment, steps, _ = self.deployment(Path(directory))
            deployment.prepare()
            steps.reset_mock()
            deployment.prepare()
            steps.backup.assert_not_called()
            steps.prepare.assert_not_called()
            steps.verify_backup.assert_called_once_with("backup-1")


if __name__ == "__main__":
    unittest.main()
