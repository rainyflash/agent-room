from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools import release, release_native
from tools.prodops.runtime import ProductionRuntime


class NativeReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifacts = []
        for name, kind in (("updater.json", "update-manifest"), ("desktop.exe", "desktop"), ("installer.exe", "installer")):
            path = self.root / name
            path.write_text("test payload", encoding="utf-8")
            self.artifacts.append(release.ArtifactSource(name, kind, "windows-x86_64", path, f"https://example.org/{name}", path, "https://example.org/sbom", path, "https://example.org/signature", "sigstore-bundle"))
        self.write("release-inventory.json", {})
        self.write("release-metadata.json", {"version": "0.2.0"})
        self.updater = {"version": "0.2.0", "platforms": {"windows-x86_64": {"url": self.artifacts[1].url, "signature": "minisign-test-only"}}}
        self.receipt = {"schemaVersion": 1, "version": "0.2.0", "result": "passed", "platform": "windows-x86_64", "installer": {"sha256": release.sha256_file(self.artifacts[2].path)}, "checks": dict.fromkeys(release_native.INSTALLER_CHECKS, True)}
        self.write("updater.json", self.updater)
        self.write("windows-installer-acceptance.json", self.receipt)
        self.write("trust.json", {"plugins": {"updater": {"pubkey": "independent-test-key"}}})

    def write(self, name, value):
        (self.root / name).write_text(json.dumps(value), encoding="utf-8")

    def verify(self):
        release_native.verify(self.root, Path("verifier"), self.root / "trust.json")

    @patch.object(release, "run_checked")
    def test_exact_updater_payload_uses_independent_key(self, run):
        def inspect_signature(args, _label):
            signature = Path(args[args.index("--signature") + 1])
            self.assertTrue(signature.is_file())
            self.assertEqual(signature.read_text(encoding="utf-8"), "minisign-test-only")
            self.assertNotEqual(signature.parent, self.root)
        run.side_effect = inspect_signature
        with patch.object(release, "parse_artifacts", return_value=self.artifacts): self.verify()
        args = run.call_args.args[0]
        self.assertIn("verify-tauri", args)
        self.assertEqual(args[args.index("--public-key") + 1], "independent-test-key")
        self.assertEqual(args[args.index("--payload") + 1], str(self.artifacts[1].path))
        self.assertFalse(Path(args[args.index("--signature") + 1]).exists())

    @patch.object(release, "run_checked")
    def test_signature_file_is_removed_when_the_native_verifier_rejects_it(self, run):
        run.side_effect = release.ReleaseFailure("invalid signature")
        with patch.object(release, "parse_artifacts", return_value=self.artifacts):
            with self.assertRaisesRegex(release.ReleaseFailure, "invalid signature"):
                self.verify()
        args = run.call_args.args[0]
        self.assertFalse(Path(args[args.index("--signature") + 1]).exists())

    @patch.object(release, "run_checked")
    def test_missing_upgrade_check_or_wrong_installer_fails(self, _run):
        with patch.object(release, "parse_artifacts", return_value=self.artifacts):
            self.receipt["checks"]["postUpgradeBridgeLaunch"] = False
            self.write("windows-installer-acceptance.json", self.receipt)
            with self.assertRaises(release.ReleaseFailure): self.verify()
            self.receipt["checks"]["postUpgradeBridgeLaunch"] = True
            self.receipt["installer"]["sha256"] = "a" * 64
            self.write("windows-installer-acceptance.json", self.receipt)
            with self.assertRaises(release.ReleaseFailure): self.verify()

    def add_macos_candidate(self):
        artifacts = list(self.artifacts)
        for name, kind in (("desktop.app.tar.gz", "desktop"), ("installer.dmg", "installer")):
            path = self.root / name
            path.write_text("mac payload", encoding="utf-8")
            artifacts.append(release.ArtifactSource(name, kind, "darwin-aarch64", path, f"https://example.org/{name}", path, "https://example.org/sbom", path, "https://example.org/signature", "sigstore-bundle"))
        self.updater["platforms"]["darwin-aarch64"] = {"url": "https://example.org/desktop.app.tar.gz", "signature": "minisign-test-only"}
        self.write("updater.json", self.updater)
        return artifacts

    def macos_receipt(self):
        return {"schemaVersion": 1, "version": "0.2.0", "result": "passed", "platform": "darwin-aarch64", "installer": {"sha256": release.sha256_file(self.root / "installer.dmg")}, "checks": dict.fromkeys(release_native.MACOS_BUNDLE_CHECKS, True)}

    @patch.object(release, "run_checked")
    def test_each_installer_platform_carries_its_own_acceptance(self, _run):
        artifacts = self.add_macos_candidate()
        with patch.object(release, "parse_artifacts", return_value=artifacts):
            # 磁盘映像在候选里，但还没有 macOS 验收回执。
            with self.assertRaises(release.ReleaseFailure): self.verify()
            self.write("macos-bundle-acceptance.json", self.macos_receipt())
            self.verify()

    @patch.object(release, "run_checked")
    def test_macos_receipt_must_match_this_disk_image_and_be_complete(self, _run):
        artifacts = self.add_macos_candidate()
        with patch.object(release, "parse_artifacts", return_value=artifacts):
            receipt = self.macos_receipt()
            receipt["checks"]["postReplaceDesktopExitedWithBridge"] = False
            self.write("macos-bundle-acceptance.json", receipt)
            with self.assertRaises(release.ReleaseFailure): self.verify()
            receipt = self.macos_receipt()
            receipt["installer"]["sha256"] = "b" * 64
            self.write("macos-bundle-acceptance.json", receipt)
            with self.assertRaises(release.ReleaseFailure): self.verify()
            receipt = self.macos_receipt()
            receipt["platform"] = "windows-x86_64"
            self.write("macos-bundle-acceptance.json", receipt)
            with self.assertRaises(release.ReleaseFailure): self.verify()

    @patch.object(release, "run_checked")
    def test_unlisted_updater_payload_is_not_accepted(self, run):
        self.updater["platforms"]["windows-x86_64"]["url"] = "https://example.org/replacement.exe"
        self.write("updater.json", self.updater)
        with patch.object(release, "parse_artifacts", return_value=self.artifacts):
            with self.assertRaises(release.ReleaseFailure): self.verify()
        run.assert_not_called()

    def test_candidate_backup_does_not_regenerate_configuration_or_keys(self):
        runtime = ProductionRuntime(Mock(), Mock())
        repository = Mock()
        with patch.object(ProductionRuntime, "prepare") as prepare, patch.object(ProductionRuntime, "validate_compose"), patch.object(ProductionRuntime, "prepare_backup_repository", return_value=repository), patch("tools.prodops.runtime.BackupCoordinator") as coordinator:
            result = runtime.backup(preserve_configuration=True)
            prepare.assert_not_called()
            self.assertEqual(result, coordinator.return_value.create.return_value)
            repository.require_headroom.assert_called_once()
            runtime.backup()
            prepare.assert_called_once_with(generate_signing_key=True)


if __name__ == "__main__": unittest.main()
