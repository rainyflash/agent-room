from __future__ import annotations

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
MACOS_WORKFLOW = ROOT / ".github" / "workflows" / "macos-self-hosted.yml"


class CiCostPolicyTests(unittest.TestCase):
    def test_deep_validation_only_runs_when_explicitly_dispatched(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")

        supply_chain = workflow[workflow.index("  supply-chain:") :]
        integration = workflow[workflow.index("  integration:") :]
        dispatch_guard = "if: ${{ github.event_name == 'workflow_dispatch' }}"
        self.assertIn(dispatch_guard, supply_chain.split("  integration:", 1)[0])
        self.assertIn(dispatch_guard, integration)

    def test_push_ci_does_not_build_release_sidecars(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("prepare:sidecar", workflow)

    def test_windows_runtime_uses_one_runner_and_one_cache_boundary(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("  windows-runtime:", workflow)
        self.assertEqual(workflow.count("runs-on: windows-latest"), 1)
        self.assertNotIn("  bridge-platforms:", workflow)
        self.assertNotIn("  desktop-platforms:", workflow)
        self.assertIn("shared-key: windows-runtime", workflow)

    def test_macos_remains_manual_and_self_hosted(self) -> None:
        workflow = MACOS_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn("runs-on: [self-hosted, macOS, ARM64]", workflow)
        self.assertNotRegex(workflow, r"runs-on:\s+macos-")


if __name__ == "__main__":
    unittest.main()
