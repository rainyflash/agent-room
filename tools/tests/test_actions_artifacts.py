from __future__ import annotations

from subprocess import CompletedProcess
import unittest
from unittest.mock import patch

from tools.actions_artifacts import (
    Artifact,
    ArtifactRetentionFailure,
    WorkflowRun,
    github_http_status,
    is_transient_github_failure,
    parse_artifacts,
    retention_decisions,
    run_gh,
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
    def test_policy_keeps_current_active_and_latest_successful_artifacts(
        self,
    ) -> None:
        values = (
            artifact(1, "sbom", "2026-08-25T00:00:00Z", 11),
            artifact(2, "sbom", "2026-08-26T00:00:00Z", 12),
            artifact(3, "failed-log", "2026-08-26T00:00:00Z", 13),
            artifact(4, "active-log", "2026-08-26T00:00:00Z", 14),
            artifact(5, "current-log", "2026-08-26T00:00:00Z", 15, "current"),
            artifact(6, "release-candidate-v1", "2026-08-20T00:00:00Z", 16),
            artifact(7, "failed-log", "2026-08-27T00:00:00Z", 17),
            artifact(8, "browser", "2026-08-25T00:00:00Z", 18),
            artifact(9, "browser", "2026-08-26T00:00:00Z", 19),
        )
        runs = {
            11: WorkflowRun(11, "completed", "success"),
            12: WorkflowRun(12, "completed", "success"),
            13: WorkflowRun(13, "completed", "failure"),
            14: WorkflowRun(14, "in_progress", None),
            15: WorkflowRun(15, "completed", "failure"),
            16: WorkflowRun(16, "completed", "failure"),
            17: WorkflowRun(17, "completed", "failure"),
            18: WorkflowRun(18, "completed", "success"),
            19: WorkflowRun(19, "completed", "failure"),
        }

        decisions = retention_decisions(values, runs, "current")
        by_identifier = {item.artifact.identifier: item for item in decisions}

        self.assertFalse(by_identifier[1].keep)
        self.assertEqual(by_identifier[1].reason, "stale")
        self.assertEqual(by_identifier[2].reason, "latest_by_name")
        self.assertFalse(by_identifier[3].keep)
        self.assertEqual(by_identifier[4].reason, "active_workflow")
        self.assertEqual(by_identifier[5].reason, "current_revision")
        self.assertFalse(by_identifier[6].keep)
        self.assertEqual(by_identifier[6].reason, "published_release_duplicate")
        self.assertEqual(by_identifier[7].reason, "latest_by_name")
        self.assertEqual(by_identifier[8].reason, "latest_successful_by_name")
        self.assertEqual(by_identifier[9].reason, "latest_by_name")

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

    def test_github_status_and_transient_failure_classification(self) -> None:
        self.assertEqual(github_http_status("gh: Server Error (HTTP 502)"), 502)
        self.assertTrue(is_transient_github_failure("gh: Server Error (HTTP 502)"))
        self.assertTrue(is_transient_github_failure("connection reset by peer"))
        self.assertFalse(is_transient_github_failure("gh: Not Found (HTTP 404)"))

    @patch("tools.actions_artifacts.time.sleep")
    @patch("tools.actions_artifacts.subprocess.run")
    def test_github_cli_retries_transient_server_failure(
        self,
        run_mock,
        sleep_mock,
    ) -> None:
        run_mock.side_effect = (
            CompletedProcess([], 1, stdout="", stderr="gh: Server Error (HTTP 502)"),
            CompletedProcess([], 0, stdout="ok", stderr=""),
        )

        self.assertEqual(run_gh(("api", "repos/example/project")), "ok")
        self.assertEqual(run_mock.call_count, 2)
        sleep_mock.assert_called_once_with(1.0)

    @patch("tools.actions_artifacts.time.sleep")
    @patch("tools.actions_artifacts.subprocess.run")
    def test_github_cli_accepts_idempotent_delete_not_found(
        self,
        run_mock,
        sleep_mock,
    ) -> None:
        run_mock.return_value = CompletedProcess(
            [], 1, stdout="", stderr="gh: Not Found (HTTP 404)"
        )

        self.assertEqual(
            run_gh(
                ("api", "--method", "DELETE", "repos/example/project/actions/artifacts/1"),
                accepted_http_statuses=frozenset({404}),
            ),
            "",
        )
        self.assertEqual(run_mock.call_count, 1)
        self.assertEqual(sleep_mock.mock_calls, [])

    @patch("tools.actions_artifacts.time.sleep")
    @patch("tools.actions_artifacts.subprocess.run")
    def test_github_cli_fails_immediately_for_non_transient_error(
        self,
        run_mock,
        sleep_mock,
    ) -> None:
        run_mock.return_value = CompletedProcess(
            [], 1, stdout="", stderr="gh: Forbidden (HTTP 403)"
        )

        with self.assertRaisesRegex(ArtifactRetentionFailure, "HTTP 403"):
            run_gh(("api", "repos/example/project"))
        self.assertEqual(run_mock.call_count, 1)
        self.assertEqual(sleep_mock.mock_calls, [])


if __name__ == "__main__":
    unittest.main()
