from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from tools.release import ReleaseFailure
from tools.release_assets import attest_image, collect_native, merge_tauri, parse_args


class ReleaseAssetTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def create_native_source(self, platform: str) -> tuple[Path, Path, Path]:
        bundle = self.root / platform / "bundle"
        bundle.mkdir(parents=True)
        archive = bundle / "Agent Room_0.1.0_x64.nsis.zip"
        archive.write_bytes(b"desktop")
        Path(f"{archive}.sig").write_text("trusted-tauri-signature", encoding="utf-8")
        bridge = self.root / platform / "agent-room-bridge.exe"
        bridge.write_bytes(b"bridge")
        plugin = self.root / platform / "plugin.zip"
        plugin.write_bytes(b"plugin")
        return bundle, bridge, plugin

    def test_collect_native_uses_exact_updater_archive_and_stable_names(self) -> None:
        bundle, bridge, plugin = self.create_native_source("windows")
        output = self.root / "candidate" / "windows"
        metadata = output / "native-metadata.json"
        args = parse_args(
            [
                "collect-native",
                "--bundle-root",
                str(bundle),
                "--bridge",
                str(bridge),
                "--plugin",
                str(plugin),
                "--output-root",
                str(output),
                "--metadata",
                str(metadata),
                "--release-base-url",
                "https://github.com/example/agent-room/releases/download/v0.1.0",
                "--version",
                "0.1.0",
                "--rust-target",
                "x86_64-pc-windows-msvc",
            ]
        )

        collect_native(args)

        document = json.loads(metadata.read_text(encoding="utf-8"))
        self.assertEqual(document["updaterTarget"], "windows-x86_64")
        self.assertEqual({item["kind"] for item in document["artifacts"]}, {
            "desktop", "bridge", "codex-plugin"
        })
        self.assertTrue(all((output / item["path"]).is_file() for item in document["artifacts"]))

    def test_collect_native_rejects_ambiguous_archives(self) -> None:
        bundle, bridge, plugin = self.create_native_source("ambiguous")
        duplicate = bundle / "second.nsis.zip"
        duplicate.write_bytes(b"duplicate")
        Path(f"{duplicate}.sig").write_text("signature", encoding="utf-8")
        args = parse_args(
            [
                "collect-native",
                "--bundle-root",
                str(bundle),
                "--bridge",
                str(bridge),
                "--plugin",
                str(plugin),
                "--output-root",
                str(self.root / "candidate"),
                "--metadata",
                str(self.root / "metadata.json"),
                "--release-base-url",
                "https://releases.example/v0.1.0",
                "--version",
                "0.1.0",
                "--rust-target",
                "x86_64-pc-windows-msvc",
            ]
        )

        with self.assertRaisesRegex(ReleaseFailure, "不唯一"):
            collect_native(args)

    def test_merge_tauri_rejects_duplicate_platform(self) -> None:
        for name in ("first", "second"):
            directory = self.root / "metadata" / name
            directory.mkdir(parents=True)
            archive = directory / f"desktop-{name}.zip"
            archive.write_bytes(b"desktop")
            signature = directory / f"desktop-{name}.zip.sig"
            signature.write_text("signature", encoding="utf-8")
            (directory / f"native-metadata-{name}.json").write_text(
                json.dumps(
                    {
                        "updaterTarget": "windows-x86_64",
                        "tauriSignaturePath": signature.name,
                        "artifacts": [
                            {
                                "name": "desktop",
                                "kind": "desktop",
                                "platform": "windows-x86_64",
                                "path": archive.name,
                                "url": f"https://releases.example/{archive.name}",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
        args = parse_args(
            [
                "merge-tauri",
                "--metadata-root",
                str(self.root / "metadata"),
                "--version",
                "0.1.0",
                "--published-at-unix-seconds",
                "1800000000",
                "--output",
                str(self.root / "tauri.json"),
            ]
        )

        with self.assertRaisesRegex(ReleaseFailure, "平台重复"):
            merge_tauri(args)

    def test_attest_image_rejects_mutable_or_mismatched_reference_before_signing(self) -> None:
        manifest = self.root / "candidate" / "control-plane.oci-manifest.json"
        manifest.parent.mkdir(parents=True)
        manifest.write_bytes(b'{"schemaVersion":2}')
        args = parse_args(
            [
                "attest-image",
                "--root",
                str(self.root),
                "--manifest-path",
                manifest.relative_to(self.root).as_posix(),
                "--image-ref",
                "ghcr.io/example/control-plane:latest",
                "--name",
                "control-plane",
                "--platform",
                "linux-amd64",
                "--descriptor",
                str(self.root / "control-plane.artifact.json"),
                "--release-base-url",
                "https://releases.example/v0.1.0",
            ]
        )

        with self.assertRaisesRegex(ReleaseFailure, "不可变"):
            attest_image(args)


if __name__ == "__main__":
    unittest.main()
