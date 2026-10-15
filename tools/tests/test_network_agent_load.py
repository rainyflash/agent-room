from __future__ import annotations

from pathlib import Path
import unittest

from tools.network_agent_load import (
    LIMIT_OVERRIDES,
    Ledger,
    cgroup_files,
    latency_summary,
    parse_arguments,
    parse_load_text,
    percentile,
    within_budget,
)
from tools.prodops.config import NETWORK_AGENT_LIMIT_FIELDS


class NetworkAgentLoadTests(unittest.TestCase):
    def test_percentile_uses_nearest_rank(self) -> None:
        ordered = [float(value) for value in range(1, 21)]
        self.assertEqual(percentile(ordered, 0.5), 10.0)
        self.assertEqual(percentile(ordered, 0.95), 19.0)
        self.assertEqual(percentile([3.0], 0.95), 3.0)
        self.assertIsNone(latency_summary([]))
        self.assertEqual(latency_summary([0.2, 0.1]), {"p50": 0.1, "p95": 0.2, "max": 0.2})

    def test_only_load_messages_are_counted(self) -> None:
        self.assertEqual(parse_load_text("load 0123456789abcdef 42"), ("0123456789abcdef", 42))
        self.assertIsNone(parse_load_text("load 0123 42"))
        self.assertIsNone(parse_load_text("hello"))
        self.assertIsNone(parse_load_text(None))

    def test_ledger_counts_each_receiver_once_and_skips_the_speaker(self) -> None:
        ledger = Ledger()
        ledger.sending("a" * 16, "!room:one", 1)
        ledger.sending("b" * 16, "!room:two", 5)
        ledger.received("a" * 16, 2, 1, "@one:server")
        ledger.received("a" * 16, 2, 1, "@one:server")
        ledger.received("a" * 16, 1, 1, "@one:server")
        ledger.received("c" * 16, 3, 1, None)
        ledger.not_sent("b" * 16, "send HTTP 429")
        summary = ledger.summary({"!room:one": 3, "!room:two": 4})
        self.assertEqual(summary["messagesSent"], 1)
        self.assertEqual(summary["deliveriesExpected"], 2)
        self.assertEqual(summary["deliveriesReceived"], 1)
        self.assertEqual(summary["errors"], {"send HTTP 429": 1})
        self.assertEqual(ledger.user_id(1), "@one:server")
        ledger.begin()
        self.assertEqual(ledger.summary({})["messagesSent"], 0)
        self.assertEqual(ledger.user_id(1), "@one:server", "学到的 Matrix 用户 ID 跨档留着")

    def test_cgroup_files_for_v2_and_v1(self) -> None:
        root = Path("/sys/fs/cgroup")
        self.assertEqual(
            cgroup_files("0::/system.slice/docker-abc.scope\n"),
            (
                root / "system.slice/docker-abc.scope/cpu.stat",
                root / "system.slice/docker-abc.scope/memory.current",
            ),
        )
        self.assertEqual(
            cgroup_files("12:memory:/docker/abc\n4:cpu,cpuacct:/docker/abc\n1:name=systemd:/docker/abc\n"),
            (
                root / "cpu,cpuacct/docker/abc/cpuacct.usage",
                root / "memory/docker/abc/memory.usage_in_bytes",
            ),
        )

    def test_budget_matches_the_design(self) -> None:
        self.assertTrue(within_budget({"synapse": 0.7, "total": 1.5}))
        self.assertFalse(within_budget({"synapse": 0.71, "total": 1.0}))
        self.assertFalse(within_budget({"synapse": 0.5, "total": 1.51}))

    def test_limit_overrides_are_the_deployment_variables(self) -> None:
        variables = {variable for _, _, variable, _, _ in NETWORK_AGENT_LIMIT_FIELDS}
        limits = {name for name in LIMIT_OVERRIDES if name != "AGENT_ROOM_DB_MAX_CONNECTIONS"}
        self.assertEqual(limits, variables)
        for name, _, variable, _, maximum in NETWORK_AGENT_LIMIT_FIELDS:
            self.assertLessEqual(int(LIMIT_OVERRIDES[variable]), maximum, name)

    def test_agent_counts_must_grow(self) -> None:
        self.assertEqual(parse_arguments(["--agents", "2", "4"]).agents, [2, 4])
        with self.assertRaises(SystemExit):
            parse_arguments(["--agents", "4", "2"])


if __name__ == "__main__":
    unittest.main()
