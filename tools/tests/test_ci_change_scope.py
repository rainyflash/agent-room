from __future__ import annotations

import unittest

from tools.ci_change_scope import docs_only, is_documentation


class CiChangeScopeTests(unittest.TestCase):
    def test_documentation_directories_and_root_markdown_are_docs(self) -> None:
        for path in ("docs/known-limitations.md", "docs/adr/0009-x.md", "specs/agent-access/alpha46-release.md",
                     "README.md", "README.zh-CN.md", "SECURITY.md"):
            with self.subTest(path=path):
                self.assertTrue(is_documentation(path))

    def test_generated_or_nested_markdown_and_code_are_not_docs(self) -> None:
        for path in ("THIRD_PARTY_NOTICES.md", "plugins/agent-room/skills/agent-room/SKILL.md",
                     "apps/agent-room-mcp/README.md", "tools/release_qa.py", ".github/workflows/ci.yml",
                     "Cargo.lock", "apps/web/src/main.tsx"):
            with self.subTest(path=path):
                self.assertFalse(is_documentation(path))

    def test_any_code_change_or_an_empty_diff_runs_everything(self) -> None:
        self.assertTrue(docs_only(["docs/a.md", "specs/b.md", "README.md"]))
        self.assertFalse(docs_only(["docs/a.md", "tools/release_qa.py"]))
        self.assertFalse(docs_only([]))
        self.assertFalse(docs_only([""]))


if __name__ == "__main__":
    unittest.main()
