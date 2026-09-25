from __future__ import annotations

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
MACOS_WORKFLOW = ROOT / ".github" / "workflows" / "macos.yml"
RELEASE_CANDIDATE_WORKFLOW = ROOT / ".github" / "workflows" / "release-candidate.yml"
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

    def test_pull_requests_catch_stale_license_inventory(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")
        steps = [line.strip() for line in job_lines(workflow, "quality")]

        # 供应链作业只在派发时运行，PR 上由必跑的 quality 检查许可证清单；而且要赶在
        # 不带 --locked 的 Cargo 编译之前，否则它们改写 Cargo.lock 后就查不到提交里的原样。
        self.assertIn("github.event_name != 'workflow_dispatch'", job_condition(workflow, "quality"))
        self.assertIn("run: python tools/license_inventory.py check", steps)
        builds = [
            index for index, step in enumerate(steps) if re.match(r"run: cargo (?:build|check|clippy|test)\b", step)
        ]
        self.assertTrue(builds)
        self.assertLess(steps.index("run: python tools/license_inventory.py check"), min(builds))

    def test_docs_only_pull_requests_skip_heavy_work_but_keep_the_required_gate(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")
        gate = "needs.changes.outputs.docs_only != 'true'"

        # quality 是必需检查：只改文档也要照常运行并报告；changes 失败时也要跑（!cancelled）。
        quality = job_condition(workflow, "quality")
        self.assertIn("!cancelled()", quality)
        self.assertNotIn("docs_only", quality)
        # 另外两个 PR 作业只在确认只改了文档时跳过；判断失败、输出为空时照常运行。
        for name in ("windows-runtime", "web-browser"):
            with self.subTest(job=name):
                condition = job_condition(workflow, name)
                self.assertIn("!cancelled()", condition)
                self.assertIn(gate, condition)
                self.assertIn("    needs: changes", job_lines(workflow, name))

        # quality 里只有编译与测试步骤挂在这个条件上；格式、许可证与 Python 工具检查照常运行。
        steps = [line.strip() for line in job_lines(workflow, "quality")]
        for run in ("run: python tools/license_inventory.py check",
                    "run: corepack pnpm@10.28.0 format:check",
                    "run: python -m unittest discover -s tools/tests -p 'test_*.py'"):
            with self.subTest(always=run):
                self.assertNotEqual(steps[steps.index(run) - 1], f"if: {gate}")
        for run in ("run: cargo clippy --workspace --all-targets --all-features -- -D warnings",
                    "run: cargo test --workspace --all-features",
                    "run: corepack pnpm@10.28.0 build",
                    "run: corepack pnpm@10.28.0 test"):
            with self.subTest(gated=run):
                self.assertEqual(steps[steps.index(run) - 1], f"if: {gate}")

    def test_release_dispatch_is_not_cancelled_by_later_pushes(self) -> None:
        workflow = CI_WORKFLOW.read_text(encoding="utf-8")
        group = next(line.strip() for line in workflow.splitlines() if line.startswith("  group: "))

        # 普通推送仍按分支取消旧运行；带操作号的发布调度自成一组，main 继续合并不会取消它。
        self.assertIn("${{ github.ref }}", group)
        self.assertIn("inputs.operation_id", group)
        self.assertIn("cancel-in-progress: true", workflow)

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

    def test_release_builds_keep_their_own_compile_cache(self) -> None:
        release = RELEASE_CANDIDATE_WORKFLOW.read_text(encoding="utf-8")
        keys = set(re.findall(r"cache_key: (\S+)", release))

        # 与 CI 的 cargo check 共用键时，发布作业完全命中 debug 产物就不再保存，
        # release 编译结果永远存不进缓存。
        self.assertEqual(keys, {"windows-release", "macos-release"})
        for other in (CI_WORKFLOW, MACOS_WORKFLOW):
            shared = set(re.findall(r"shared-key: (\S+)", other.read_text(encoding="utf-8")))
            self.assertFalse(keys & shared, other.name)

    def test_macos_stays_manual_on_a_standard_runner(self) -> None:
        workflow = MACOS_WORKFLOW.read_text(encoding="utf-8")

        # 公开仓库的标准 runner 不计费，因此 macOS 不再需要自托管机器；但更大规格的
        # runner 即使公开仓库也要付费，而且 macOS 构建很慢，仍然只按需手动派发。
        self.assertIn("workflow_dispatch:", workflow)
        self.assertNotIn("pull_request:", workflow)
        self.assertNotIn("push:", workflow)
        runners = [line.strip() for line in workflow.splitlines() if line.strip().startswith("runs-on:")]
        self.assertEqual(len(runners), 1)
        self.assertRegex(runners[0], r"^runs-on: macos-(?:latest|[0-9]+)$")


if __name__ == "__main__":
    unittest.main()
