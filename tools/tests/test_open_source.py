from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from tools.open_source import (
    OpenSourceValidationError,
    validate_markdown_links,
    validate_reserved_example,
)


class OpenSourceValidationTests(unittest.TestCase):
    def test_local_markdown_links_must_exist_and_stay_in_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = root / "README.md"
            target = root / "docs" / "guide.md"
            target.parent.mkdir()
            target.write_text("# Guide\n", encoding="utf-8")
            document.write_text("[Guide](./docs/guide.md)\n", encoding="utf-8")
            validate_markdown_links((document,), root)

            document.write_text("[Missing](./docs/missing.md)\n", encoding="utf-8")
            with self.assertRaises(OpenSourceValidationError):
                validate_markdown_links((document,), root)

    def test_production_example_accepts_only_reserved_hosts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "deployment.example.json"
            value = {
                "public": {
                    "serverName": "room.example.com",
                    "appDomain": "app.room.example.com",
                    "acmeEmail": "operator@example.com",
                },
                "database": {"host": "postgres.room.example.com"},
                "objectStore": {"endpoint": "https://objects.room.example.com"},
            }
            path.write_text(json.dumps(value), encoding="utf-8")
            validate_reserved_example(path)

            value["database"]["host"] = "developer-vps.invalid-real-domain.net"
            path.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaises(OpenSourceValidationError):
                validate_reserved_example(path)

    def test_production_example_rejects_secret_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "deployment.example.json"
            path.write_text(
                json.dumps({"public": {"serverName": "room.example"}, "apiToken": "fake"}),
                encoding="utf-8",
            )
            with self.assertRaises(OpenSourceValidationError):
                validate_reserved_example(path)


if __name__ == "__main__":
    unittest.main()
