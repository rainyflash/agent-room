from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.source_revision import SourceRevisionFailure
from tools.capacity import (
    CapacityFailure,
    MAX_BRIDGE_RSS_BYTES,
    REQUIRED_REAL_SCENARIOS,
    SOAK_SECONDS,
    bridge_soak_report,
    evaluate_gate,
    percentile,
    require_git_revision,
    run_model,
    validate_bridge_executable,
)


class CapacityMathTests(unittest.TestCase):
    def test_percentile_uses_nearest_rank_and_rejects_empty_samples(self) -> None:
        self.assertEqual(percentile([4.0, 1.0, 3.0, 2.0], 0.5), 2.0)
        self.assertEqual(percentile([4.0, 1.0, 3.0, 2.0], 0.95), 4.0)
        with self.assertRaisesRegex(CapacityFailure, "不能为空"):
            percentile([], 0.95)

    @patch("tools.capacity.git_revision", return_value="revision")
    @patch("tools.capacity.require_clean_git_revision")
    @patch("tools.capacity.write_json")
    def test_model_is_reproducible_but_never_release_eligible(
        self,
        _write_json: object,
        _require_revision: object,
        _git_revision: object,
    ) -> None:
        report = run_model(39)
        self.assertTrue(report["passed"])
        self.assertEqual(report["evidenceLevel"], "model_only")
        self.assertFalse(report["releaseGateEligible"])
        metrics = report["metrics"]
        self.assertEqual(metrics["allocatedInstances"], 1_000)
        self.assertEqual(metrics["maximumRoomLoad"], 250)

    def test_bridge_soak_rejects_wrappers_and_hashes_exact_binary(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wrapper = root / "python.exe"
            wrapper.write_bytes(b"wrapper")
            with self.assertRaisesRegex(CapacityFailure, "拒绝包装器"):
                validate_bridge_executable(str(wrapper))

            bridge = root / "agent-room-bridge.exe"
            bridge.write_bytes(b"bridge-binary")
            executable, digest = validate_bridge_executable(str(bridge))

        self.assertEqual(executable.name, "agent-room-bridge.exe")
        self.assertEqual(
            digest,
            "b84cbf5e143f70b8261ccd04262c8775289de0aab71021ad1454589473745f49",
        )

    @patch("tools.capacity.git_revision", return_value="revision")
    def test_bridge_soak_gate_requires_duration_session_and_memory_budget(
        self, _git_revision: object
    ) -> None:
        common = {
            "completed": True,
            "executable": Path("agent-room-bridge.exe"),
            "executable_digest": "a" * 64,
            "process_id": 42,
            "started_at": datetime.now(UTC),
            "revision": "revision",
        }
        accepted = bridge_soak_report(
            elapsed_seconds=SOAK_SECONDS,
            samples=[{"rssBytes": 64 * 1_024 * 1_024}],
            active_session_configured=True,
            **common,
        )
        leaking = bridge_soak_report(
            elapsed_seconds=SOAK_SECONDS,
            samples=[{"rssBytes": MAX_BRIDGE_RSS_BYTES + 1}],
            active_session_configured=True,
            **common,
        )
        observer_only = bridge_soak_report(
            elapsed_seconds=SOAK_SECONDS,
            samples=[{"rssBytes": 64 * 1_024 * 1_024}],
            active_session_configured=False,
            **common,
        )

        self.assertTrue(accepted["releaseGateEligible"])
        self.assertFalse(leaking["passed"])
        self.assertFalse(observer_only["passed"])

    @patch(
        "tools.capacity.require_clean_git_revision",
        side_effect=SourceRevisionFailure(
            "Git 修订在验收期间发生变化：开始 starting-revision，结束 ending-revision。"
        ),
    )
    def test_revision_guard_rejects_source_changes_during_acceptance(
        self, _require_revision: object
    ) -> None:
        with self.assertRaisesRegex(CapacityFailure, "开始 starting-revision"):
            require_git_revision("starting-revision")


class CapacityGateTests(unittest.TestCase):
    @patch("tools.capacity.git_revision", return_value="revision")
    @patch("tools.capacity.require_clean_git_revision")
    @patch("tools.capacity.write_json")
    def test_gate_rejects_missing_model_and_stale_evidence(
        self,
        _write_json: object,
        _require_revision: object,
        _git_revision: object,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model = root / "model.json"
            model.write_text(
                json.dumps(
                    {
                        "scenario": REQUIRED_REAL_SCENARIOS[0],
                        "revision": "revision",
                        "evidenceLevel": "model_only",
                        "passed": True,
                        "releaseGateEligible": True,
                    }
                ),
                encoding="utf-8",
            )
            stale = root / "stale.json"
            stale.write_text(
                json.dumps(
                    {
                        "scenario": REQUIRED_REAL_SCENARIOS[1],
                        "revision": "old",
                        "evidenceLevel": "real_service",
                        "passed": True,
                        "releaseGateEligible": True,
                    }
                ),
                encoding="utf-8",
            )
            report = evaluate_gate([model, stale])

        self.assertFalse(report["passed"])
        failures = "\n".join(report["failures"])
        self.assertIn("没有真实依赖证据", failures)
        self.assertIn("不是当前 Git 修订", failures)
        self.assertIn("缺少场景 bridge_72_hour_soak", failures)

    @patch("tools.capacity.git_revision", return_value="revision")
    @patch("tools.capacity.require_clean_git_revision")
    @patch("tools.capacity.write_json")
    def test_gate_accepts_only_complete_current_real_evidence(
        self,
        _write_json: object,
        _require_revision: object,
        _git_revision: object,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            paths: list[Path] = []
            for scenario in REQUIRED_REAL_SCENARIOS:
                path = Path(directory) / f"{scenario}.json"
                path.write_text(
                    json.dumps(
                        {
                            "scenario": scenario,
                            "revision": "revision",
                            "evidenceLevel": "real_service",
                            "passed": True,
                            "releaseGateEligible": True,
                        }
                    ),
                    encoding="utf-8",
                )
                paths.append(path)
            report = evaluate_gate(paths)

        self.assertTrue(report["passed"])
        self.assertEqual(report["failures"], [])


if __name__ == "__main__":
    unittest.main()
