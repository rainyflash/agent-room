from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.capacity import (
    CapacityFailure,
    REQUIRED_REAL_SCENARIOS,
    evaluate_gate,
    percentile,
    run_model,
)


class CapacityMathTests(unittest.TestCase):
    def test_percentile_uses_nearest_rank_and_rejects_empty_samples(self) -> None:
        self.assertEqual(percentile([4.0, 1.0, 3.0, 2.0], 0.5), 2.0)
        self.assertEqual(percentile([4.0, 1.0, 3.0, 2.0], 0.95), 4.0)
        with self.assertRaisesRegex(CapacityFailure, "不能为空"):
            percentile([], 0.95)

    @patch("tools.capacity.git_revision", return_value="revision")
    @patch("tools.capacity.write_json")
    def test_model_is_reproducible_but_never_release_eligible(
        self, _write_json: object, _git_revision: object
    ) -> None:
        report = run_model(39)
        self.assertTrue(report["passed"])
        self.assertEqual(report["evidenceLevel"], "model_only")
        self.assertFalse(report["releaseGateEligible"])
        metrics = report["metrics"]
        self.assertEqual(metrics["allocatedInstances"], 1_000)
        self.assertEqual(metrics["maximumRoomLoad"], 250)


class CapacityGateTests(unittest.TestCase):
    @patch("tools.capacity.git_revision", return_value="revision")
    @patch("tools.capacity.write_json")
    def test_gate_rejects_missing_model_and_stale_evidence(
        self, _write_json: object, _git_revision: object
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
    @patch("tools.capacity.write_json")
    def test_gate_accepts_only_complete_current_real_evidence(
        self, _write_json: object, _git_revision: object
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
