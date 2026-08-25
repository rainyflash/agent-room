from __future__ import annotations

import unittest

from tools.sensitive_output import OUTPUT_CHANNELS, scan_text


class SensitiveOutputTests(unittest.TestCase):
    def test_五类最小结构化输出通过(self) -> None:
        samples = {
            "log": '{"code":"matrix.unavailable","correlation.id":"01990d9e"}',
            "metric": 'http_request_duration_seconds{route="/rooms/:id",status="200"} 0.12',
            "error": '{"code":"content.invalid","correlationId":"01990d9e"}',
            "audit": '{"action":"moderation.report.created","outcome":"allowed"}',
            "crash": '{"module":"bridge_core::sync","location":"incoming.rs:255"}',
        }

        self.assertEqual(set(samples), set(OUTPUT_CHANNELS))
        for channel, text in samples.items():
            with self.subTest(channel=channel):
                self.assertEqual(scan_text(channel, text), ())

    def test_五类输出都会拒绝已知秘密(self) -> None:
        for channel in OUTPUT_CHANNELS:
            with self.subTest(channel=channel):
                violations = scan_text(
                    channel,
                    "prefix task-35-canary-secret suffix",
                    known_secrets=("task-35-canary-secret",),
                )
                self.assertEqual({item.rule for item in violations}, {"known-secret"})

    def test_通用凭据形态不会被原样放行(self) -> None:
        fixtures = (
            "Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
            "access_token=raw-token-value",
            "https://operator:password@example.test/path",
            "-----BEGIN PRIVATE KEY-----",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop",
        )

        for fixture in fixtures:
            with self.subTest(fixture=fixture[:16]):
                self.assertTrue(scan_text("log", fixture))

    def test_未知输出通道明确失败(self) -> None:
        with self.assertRaises(ValueError):
            scan_text("trace", "safe")


if __name__ == "__main__":
    unittest.main()
