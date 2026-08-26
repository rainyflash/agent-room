from __future__ import annotations

import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from tools.license_inventory import (
    LicenseInventoryError,
    build_inventory,
    parse_cargo_metadata,
    parse_pnpm_licenses,
    platform_specific_pnpm_keys,
    render_markdown,
)


class LicenseInventoryTests(unittest.TestCase):
    def test_parsers_exclude_workspace_packages_and_local_paths(self) -> None:
        cargo = parse_cargo_metadata(
            {
                "packages": [
                    {
                        "name": "agent-room-domain",
                        "version": "0.1.0",
                        "license": "MIT",
                        "source": None,
                    },
                    {
                        "name": "serde",
                        "version": "1.0.0",
                        "license": "MIT OR Apache-2.0",
                        "source": "registry+https://github.com/rust-lang/crates.io-index",
                        "repository": "https://github.com/serde-rs/serde",
                    },
                ]
            }
        )
        npm = parse_pnpm_licenses(
            {
                "MIT": [
                    {
                        "name": "@agent-room/web",
                        "versions": ["0.1.0"],
                        "paths": ["C:/private/workspace"],
                        "license": "MIT",
                    },
                    {
                        "name": "react",
                        "versions": ["19.2.0"],
                        "paths": ["C:/private/workspace/node_modules/react"],
                        "license": "MIT",
                    },
                ]
            }
        )

        self.assertEqual([record.name for record in cargo], ["serde"])
        self.assertEqual([record.name for record in npm], ["react"])
        self.assertNotIn("private", npm[0].source)

    def test_inventory_and_markdown_are_deterministic(self) -> None:
        cargo = parse_cargo_metadata(
            {
                "packages": [
                    {
                        "name": "zeta",
                        "version": "1.0.0",
                        "license": "MIT",
                        "source": "registry+index",
                        "repository": None,
                    }
                ]
            }
        )
        npm = parse_pnpm_licenses(
            {
                "Apache-2.0": [
                    {"name": "alpha", "versions": ["2.0.0"], "license": "Apache-2.0"}
                ]
            }
        )
        inventory = build_inventory(
            cargo,
            npm,
            cargo_lock_digest="a" * 64,
            pnpm_lock_digest="b" * 64,
        )

        self.assertEqual(
            [entry["name"] for entry in inventory["dependencies"]],
            ["zeta", "alpha"],
        )
        markdown = render_markdown(inventory)
        self.assertIn("Cargo packages: 1", markdown)
        self.assertIn("npm packages: 1", markdown)
        self.assertIn("[alpha](https://www.npmjs.com/package/alpha/v/2.0.0)", markdown)

    def test_missing_license_fails_loudly(self) -> None:
        with self.assertRaises(LicenseInventoryError):
            parse_cargo_metadata(
                {
                    "packages": [
                        {
                            "name": "unknown",
                            "version": "1.0.0",
                            "license": None,
                            "source": "registry+index",
                        }
                    ]
                }
            )

    def test_platform_specific_packages_are_excluded_from_root_inventory(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            common = root / "node_modules" / ".pnpm" / "common" / "node_modules" / "common"
            binary = root / "node_modules" / ".pnpm" / "binary" / "node_modules" / "binary-linux"
            common.mkdir(parents=True)
            binary.mkdir(parents=True)
            (common / "package.json").write_text(
                json.dumps({"name": "common", "version": "1.0.0", "license": "MIT"}),
                encoding="utf-8",
            )
            (binary / "package.json").write_text(
                json.dumps(
                    {
                        "name": "binary-linux",
                        "version": "2.0.0",
                        "license": "MIT",
                        "os": ["linux"],
                        "cpu": ["x64"],
                    }
                ),
                encoding="utf-8",
            )
            payload = {
                "MIT": [
                    {
                        "name": "common",
                        "versions": ["1.0.0"],
                        "paths": [str(common)],
                        "license": "MIT",
                    },
                    {
                        "name": "binary-linux",
                        "versions": ["2.0.0"],
                        "paths": [str(binary)],
                        "license": "MIT",
                    },
                ]
            }

            excluded = platform_specific_pnpm_keys(payload, workspace_root=root)
            records = parse_pnpm_licenses(payload, excluded=excluded)

            self.assertEqual(excluded, frozenset({("binary-linux", "2.0.0")}))
            self.assertEqual([record.name for record in records], ["common"])


if __name__ == "__main__":
    unittest.main()
