from __future__ import annotations

import unittest

from tools.capacity_federation import delivery_assessment


class FederationCapacityEvidenceTests(unittest.TestCase):
    def test_short_real_smoke_can_pass_functionally_but_not_release_gate(self) -> None:
        report = delivery_assessment(
            ["$one", "$two"],
            ["$one", "$two"],
            [],
            actual_outage_seconds=5.01,
            requested_outage_seconds=5,
        )

        self.assertTrue(report["passed"])
        self.assertFalse(report["releaseGateEligible"])

    def test_release_gate_requires_full_duration_volume_order_and_uniqueness(self) -> None:
        expected = [f"$event-{index}" for index in range(10)]
        accepted = delivery_assessment(
            expected,
            expected,
            [],
            actual_outage_seconds=1_800.1,
            requested_outage_seconds=1_800,
        )
        reordered = delivery_assessment(
            expected,
            list(reversed(expected)),
            [expected[0]],
            actual_outage_seconds=1_800.1,
            requested_outage_seconds=1_800,
        )

        self.assertTrue(accepted["releaseGateEligible"])
        self.assertFalse(reordered["passed"])
        self.assertFalse(reordered["eventOrderPreserved"])
        self.assertEqual(reordered["duplicateEvents"], 1)


if __name__ == "__main__":
    unittest.main()
