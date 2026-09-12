from __future__ import annotations

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
MACOS_WORKFLOW = ROOT / ".github" / "workflows" / "macos-self-hosted.yml"
JOB_HEADING = re.compile(r"^  [a-z0-9-]+:$")


def job_lines(workflow: str, name: str) -> list[str]:
    """截取单个作业，避免把后续作业的条件误算进来。"""

    lines = workflow.splitlines()
    heading = f"  {name}:"
    if heading not in lines:
        raise AssertionError(f"ci.yml 缺少作业 {name}")
    start = lines.index(heading)
    end = next(
        (index for index in range(start + 1, len(lines)) if JOB_HEADING.fullmatch(lines[index])),
        len(lines),
    )
    return lines[start:end]


def job_condition(workflow: str, name: str) -> str:
    """返回作业级 `if:`；步骤级条件缩进更深，不会被算入。"""

    conditions = [
        line.strip() for line in job_lines(workflow, name) if line.startswith("    if:")
    ]
    if len(conditions) != 1:
        raise AssertionError(f"作业 {name} 必须恰好声明一个作业级条件")
    return conditions[0]


class CiCostPolicyTests(unittest.TestCase):
    def test_deep_validation_only_runs_when_explicitly_dispatched(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")

        for name in ("supply-chain", "integration"):
            with self.subTest(job=name):
                condition = job_condition(workflow, name)
                # 允许继续收窄（例如只在 suite=all 时运行），但不得放宽到 push 或 pull_request。
                self.assertIn("github.event_name == 'workflow_dispatch'", condition)
                self.assertNotIn("||", condition)

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
