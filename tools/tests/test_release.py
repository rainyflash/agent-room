from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.release import (
    CandidatePaths,
    ReleaseFailure,
    build_candidate,
    create_descriptor,
    create_inventory,
    parse_args,
    prepare,
    validate_candidate_files,
)


class ReleaseCandidateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write_json(self, path: Path, value: object) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding="utf-8")

    def write_bundle(self, path: Path) -> None:
        self.write_json(
            path,
            {
                "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
                "verificationMaterial": {},
                "messageSignature": {},
            },
        )

    def inventory(self) -> Path:
        artifacts = []
        definitions = (
            ("control-plane", "oci-image", "linux-amd64", "oci"),
            ("identity", "oci-image", "linux-amd64", "oci"),
            ("web", "oci-image", "linux-amd64", "oci"),
            ("bridge", "bridge", "windows-x86_64", "blob"),
            ("desktop", "desktop", "windows-x86_64", "blob"),
            ("codex-plugin", "codex-plugin", "all", "blob"),
            ("tauri-update", "update-manifest", "windows-x86_64", "blob"),
        )
        for name, kind, platform, signature_mode in definitions:
            artifact_path = self.root / "artifacts" / f"{name}.bin"
            artifact_path.parent.mkdir(parents=True, exist_ok=True)
            artifact_path.write_bytes(f"artifact:{name}".encode())
            digest = hashlib.sha256(artifact_path.read_bytes()).hexdigest()
            sbom_path = self.root / "sbom" / f"{name}.cdx.json"
            self.write_json(
                sbom_path,
                {"bomFormat": "CycloneDX", "specVersion": "1.6", "components": []},
            )
            signature_path = self.root / "signatures" / f"{name}.sigstore.json"
            self.write_bundle(signature_path)
            url = (
                f"oci://ghcr.io/example/{name}@sha256:{digest}"
                if kind == "oci-image"
                else f"https://releases.example/{name}.bin"
            )
            artifacts.append(
                {
                    "name": name,
                    "kind": kind,
                    "platform": platform,
                    "path": artifact_path.relative_to(self.root).as_posix(),
                    "url": url,
                    "sbomPath": sbom_path.relative_to(self.root).as_posix(),
                    "sbomUrl": f"https://releases.example/{name}.cdx.json",
                    "signaturePath": signature_path.relative_to(self.root).as_posix(),
                    "signatureUrl": f"https://releases.example/{name}.sigstore.json",
                    "signatureMode": signature_mode,
                }
            )
        inventory = self.root / "inventory.json"
        self.write_json(
            inventory,
            {
                "schemaVersion": 1,
                "channel": "testing",
                "sequence": 41,
                "version": "0.2.0",
                "publishedAtUnixSeconds": 1_800_000_000,
                "expiresAtUnixSeconds": 1_800_086_400,
                "rollbackFrom": None,
                "tauriManifestUrl": "https://releases.example/tauri.json",
                "artifacts": artifacts,
            },
        )
        return inventory

    def test_prepare_builds_complete_draft_and_evidence(self) -> None:
        inventory = self.inventory()
        paths = CandidatePaths(self.root / "release.json", self.root / "evidence.json")

        prepare(self.root, inventory, paths)

        manifest = json.loads(paths.manifest.read_text(encoding="utf-8"))
        evidence = json.loads(paths.evidence.read_text(encoding="utf-8"))
        self.assertEqual({item["kind"] for item in manifest["artifacts"]}, {
            "oci-image", "bridge", "desktop", "codex-plugin", "update-manifest"
        })
        self.assertEqual(
            evidence["manifestSha256"],
            hashlib.sha256(paths.manifest.read_bytes()).hexdigest(),
        )

    def test_artifact_path_cannot_escape_candidate_root(self) -> None:
        inventory_path = self.inventory()
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["artifacts"][0]["path"] = "../outside.bin"
        self.write_json(inventory_path, inventory)

        with self.assertRaisesRegex(ReleaseFailure, "相对路径"):
            build_candidate(self.root, inventory_path)

    def test_missing_sbom_or_signature_is_rejected(self) -> None:
        inventory_path = self.inventory()
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["artifacts"][1]["sbomPath"] = "sbom/missing.cdx.json"
        self.write_json(inventory_path, inventory)

        with self.assertRaisesRegex(ReleaseFailure, "不存在"):
            build_candidate(self.root, inventory_path)

    def test_duplicate_artifact_identity_is_rejected(self) -> None:
        inventory_path = self.inventory()
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        inventory["artifacts"].append(dict(inventory["artifacts"][0]))
        self.write_json(inventory_path, inventory)

        with self.assertRaisesRegex(ReleaseFailure, "身份重复"):
            build_candidate(self.root, inventory_path)

    def test_changed_artifact_changes_candidate_digest(self) -> None:
        inventory_path = self.inventory()
        paths = CandidatePaths(self.root / "release.json", self.root / "evidence.json")
        prepare(self.root, inventory_path, paths)
        artifact = self.root / "artifacts" / "bridge.bin"
        artifact.write_bytes(b"tampered")

        with self.assertRaisesRegex(ReleaseFailure, "不一致"):
            validate_candidate_files(self.root, inventory_path, paths)

    def test_descriptor_and_inventory_commands_preserve_safe_relative_paths(self) -> None:
        source_inventory = json.loads(self.inventory().read_text(encoding="utf-8"))
        descriptor_root = self.root / "descriptors"
        for index, artifact in enumerate(source_inventory["artifacts"]):
            args = parse_args(
                [
                    "descriptor",
                    "--root",
                    str(self.root),
                    "--output",
                    str(descriptor_root / f"{index}.artifact.json"),
                    "--name",
                    artifact["name"],
                    "--kind",
                    artifact["kind"],
                    "--platform",
                    artifact["platform"],
                    "--path",
                    artifact["path"],
                    "--url",
                    artifact["url"],
                    "--sbom-path",
                    artifact["sbomPath"],
                    "--sbom-url",
                    artifact["sbomUrl"],
                    "--signature-path",
                    artifact["signaturePath"],
                    "--signature-url",
                    artifact["signatureUrl"],
                    "--signature-mode",
                    artifact["signatureMode"],
                ]
            )
            create_descriptor(args)

        output = self.root / "assembled.json"
        inventory_args = parse_args(
            [
                "inventory",
                "--root",
                str(self.root),
                "--descriptors",
                str(descriptor_root),
                "--output",
                str(output),
                "--channel",
                "testing",
                "--sequence",
                "41",
                "--version",
                "0.2.0",
                "--published-at-unix-seconds",
                "1800000000",
                "--expires-at-unix-seconds",
                "1800086400",
                "--tauri-manifest-url",
                "https://releases.example/tauri.json",
            ]
        )
        create_inventory(inventory_args)

        assembled = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(len(assembled["artifacts"]), 7)
        self.assertTrue(all(not Path(item["path"]).is_absolute() for item in assembled["artifacts"]))


if __name__ == "__main__":
    unittest.main()
