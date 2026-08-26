from __future__ import annotations

import unittest

from tools.actions_artifacts import (
    Artifact,
    ArtifactRetentionFailure,
    WorkflowRun,
    parse_artifacts,
    retention_decisions,
)


def artifact(
    identifier: int,
    name: str,
    created_at: str,
    run_id: int,
    head_sha: str = "old",
) -> Artifact:
    return Artifact(
        identifier=identifier,
        name=name,
        size_bytes=identifier * 100,
        created_at=created_at,
        run_id=run_id,
        head_sha=head_sha,
    )


class ArtifactRetentionPolicyTests(unittest.TestCase):
    def test_policy_keeps_release_current_active_and_latest_successful_artifacts(
        self,
    ) -> None:
        values = (
            artifact(1, "sbom", "2026-08-25T00:00:00Z", 11),
            artifact(2, "sbom", "2026-08-26T00:00:00Z", 12),
            artifact(3, "failed-log", "2026-08-26T00:00:00Z", 13),
            artifact(4, "active-log", "2026-08-26T00:00:00Z", 14),
            artifact(5, "current-log", "2026-08-26T00:00:00Z", 15, "current"),
            artifact(6, "release-candidate-v1", "2026-08-20T00:00:00Z", 16),
        )
        runs = {
            11: WorkflowRun(11, "completed", "success"),
            12: WorkflowRun(12, "completed", "success"),
            13: WorkflowRun(13, "completed", "failure"),
            14: WorkflowRun(14, "in_progress", None),
            15: WorkflowRun(15, "completed", "failure"),
            16: WorkflowRun(16, "completed", "failure"),
        }

        decisions = retention_decisions(values, runs, "current")
        by_identifier = {item.artifact.identifier: item for item in decisions}

        self.assertFalse(by_identifier[1].keep)
        self.assertEqual(by_identifier[1].reason, "stale")
        self.assertEqual(by_identifier[2].reason, "latest_successful_by_name")
        self.assertFalse(by_identifier[3].keep)
        self.assertEqual(by_identifier[4].reason, "active_workflow")
        self.assertEqual(by_identifier[5].reason, "current_revision")
        self.assertEqual(by_identifier[6].reason, "release_artifact")

    def test_policy_fails_closed_when_run_truth_is_missing(self) -> None:
        with self.assertRaisesRegex(ArtifactRetentionFailure, "缺少 Workflow Run"):
            retention_decisions(
                (artifact(1, "sbom", "2026-08-26T00:00:00Z", 11),),
                {},
                "current",
            )

    def test_artifact_parser_accepts_paginated_github_shape(self) -> None:
        values = parse_artifacts(
            [
                {
                    "artifacts": [
                        {
                            "id": 42,
                            "name": "sbom",
                            "size_in_bytes": 1024,
                            "created_at": "2026-08-26T00:00:00Z",
                            "workflow_run": {"id": 99, "head_sha": "a" * 40},
                        }
                    ]
                }
            ]
        )

        self.assertEqual(values[0].identifier, 42)
        self.assertEqual(values[0].run_id, 99)


if __name__ == "__main__":
    unittest.main()
