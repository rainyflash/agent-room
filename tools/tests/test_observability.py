from __future__ import annotations

import unittest

from tools.observability import (
    DRILL_TARGETS,
    EXPECTED_ALERTS,
    ObservabilityError,
    _dashboard_expressions,
    _validate_low_cardinality_labels,
)


class ObservabilityContractTests(unittest.TestCase):
    def test_fault_targets_cover_every_required_failure_domain(self) -> None:
        self.assertEqual(
            set(DRILL_TARGETS),
            {"control-plane", "matrix", "object-store", "oidc", "federation", "bridge"},
        )
        self.assertEqual(len(EXPECTED_ALERTS), 13)

    def test_sensitive_or_high_cardinality_rule_labels_are_rejected(self) -> None:
        for label in ("user_id", "room", "message_digest", "local_path", "token_kind"):
            with self.assertRaises(ObservabilityError):
                _validate_low_cardinality_labels(f'expr: metric{{{label}="value"}}')

    def test_dashboard_expression_parser_rejects_invalid_panel_shape(self) -> None:
        with self.assertRaises(ObservabilityError):
            _dashboard_expressions({"panels": ["not-an-object"]})


if __name__ == "__main__":
    unittest.main()
