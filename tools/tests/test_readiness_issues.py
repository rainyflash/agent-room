from __future__ import annotations

import unittest

from tools.readiness_issues import (
    BlockerIssue,
    ReadinessIssueError,
    issue_title,
    parse_existing_issues,
    render_body,
)


class ReadinessIssueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.blocker = BlockerIssue(
            identifier="GNG-007",
            title="缺少外部复现",
            owner="外部贡献者",
            exit_condition="从干净检出完成启动。",
            evidence=("README.md",),
        )

    def test_body_uses_marker_and_immutable_evidence_link(self) -> None:
        revision = "a" * 40
        body = render_body(self.blocker, "owner/repo", revision)

        self.assertEqual(issue_title(self.blocker), "[GNG-007] 缺少外部复现")
        self.assertIn("<!-- agent-room-go-no-go:GNG-007 -->", body)
        self.assertIn(f"https://github.com/owner/repo/blob/{revision}/README.md", body)

    def test_existing_issue_parser_ignores_unmanaged_issues(self) -> None:
        issues = parse_existing_issues(
            [
                {"number": 1, "state": "OPEN", "body": "普通 Issue"},
                {
                    "number": 2,
                    "state": "closed",
                    "body": "<!-- agent-room-go-no-go:GNG-007 -->",
                },
            ]
        )

        self.assertEqual(len(issues), 1)
        self.assertEqual(issues[0].identifier, "GNG-007")
        self.assertEqual(issues[0].state, "CLOSED")

    def test_duplicate_managed_issue_fails_loudly(self) -> None:
        duplicate = {
            "number": 1,
            "state": "OPEN",
            "body": "<!-- agent-room-go-no-go:GNG-001 -->",
        }
        with self.assertRaisesRegex(ReadinessIssueError, "重复"):
            parse_existing_issues([duplicate, {**duplicate, "number": 2}])


if __name__ == "__main__":
    unittest.main()
