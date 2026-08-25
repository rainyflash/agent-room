from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.event_namespace import (
    ACTIVE_NAMESPACE,
    LEGACY_NAMESPACE,
    legacy_occurrences,
    migrate,
)


class EventNamespaceTests(unittest.TestCase):
    def test_migrate_rewrites_supported_text_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "event.ts"
            source.write_text(
                f"export const type = '{LEGACY_NAMESPACE}.message.preview.v1';\n",
                encoding="utf-8",
            )

            changed = migrate(root)

            self.assertEqual(changed, [source])
            self.assertIn(ACTIVE_NAMESPACE, source.read_text(encoding="utf-8"))
            self.assertEqual(legacy_occurrences(root), [])

    def test_ignored_build_output_is_not_migrated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            generated = root / "target" / "generated.rs"
            generated.parent.mkdir()
            generated.write_text(LEGACY_NAMESPACE, encoding="utf-8")

            self.assertEqual(legacy_occurrences(root), [])


if __name__ == "__main__":
    unittest.main()
