from __future__ import annotations

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release-publish.yml"


class ReleasePublishWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_草稿发布不依赖尚未创建的_git_tag(self) -> None:
        self.assertNotIn('git rev-list -n 1 "$TAG"', self.workflow)
        self.assertIn("candidate/release-metadata.json", self.workflow)

    def test_候选元数据绑定标签版本和真实_git_commit(self) -> None:
        self.assertIn(".tag | select(type == \"string\")", self.workflow)
        self.assertIn(".version | select(type == \"string\")", self.workflow)
        self.assertIn('test("^[0-9a-f]{40}$")', self.workflow)
        self.assertIn('git rev-parse --verify "$REVISION^{commit}"', self.workflow)


if __name__ == "__main__":
    unittest.main()
