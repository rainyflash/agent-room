from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from tools.release_ci import ReleaseCiFailure, append_github_output, build_metadata, parse_args


class ReleaseCiTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.manifest = self.root / "Cargo.toml"
        self.manifest.write_text(
            '[workspace]\n[workspace.package]\nversion = "0.2.0"\n',
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def arguments(self, tag: str = "v0.2.0"):
        return parse_args(
            [
                "--tag",
                tag,
                "--channel",
                "testing",
                "--sequence",
                "42",
                "--revision",
                "a" * 40,
                "--repository",
                "example/agent-room",
                "--workspace-manifest",
                str(self.manifest),
                "--metadata",
                str(self.root / "metadata.json"),
                "--promotion",
                str(self.root / "promotion.json"),
                "--now-unix-seconds",
                "1800000000",
            ]
        )

    def test_metadata_is_reproducible_and_channel_urls_are_tag_pinned(self) -> None:
        metadata = build_metadata(self.arguments())

        self.assertEqual(metadata["version"], "0.2.0")
        self.assertEqual(metadata["expiresAtUnixSeconds"], 1_800_604_800)
        self.assertEqual(
            metadata["tauriManifestUrl"],
            "https://github.com/example/agent-room/releases/download/v0.2.0/agent-room-update-v0.2.0.json",
        )

    def test_tag_must_match_workspace_version(self) -> None:
        with self.assertRaisesRegex(ReleaseCiFailure, "不一致"):
            build_metadata(self.arguments("v0.3.0"))

    def test_github_outputs_do_not_emit_json_null(self) -> None:
        metadata = build_metadata(self.arguments())
        output = self.root / "github-output.txt"

        append_github_output(output, metadata)

        lines = output.read_text(encoding="utf-8").splitlines()
        self.assertIn("rollback_from=", lines)
        self.assertNotIn("rollback_from=None", lines)
        json.dumps(metadata)


if __name__ == "__main__":
    unittest.main()
