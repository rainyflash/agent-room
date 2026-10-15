from __future__ import annotations

from pathlib import Path
import unittest

from tools.network_agent_load import (
    LIMIT_OVERRIDES,
    LOAD_NAME,
    Growth,
    Ledger,
    cgroup_files,
    create_backoff,
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

    def test_only_load_messages_and_names_are_recognized(self) -> None:
        self.assertEqual(parse_load_text("load 0123456789abcdef 42"), ("0123456789abcdef", 42))
        self.assertIsNone(parse_load_text("load 0123 42"))
        self.assertIsNone(parse_load_text("hello"))
        self.assertIsNone(parse_load_text(None))
        match = LOAD_NAME.search("Load 007 2")
        self.assertIsNotNone(match)
        assert match is not None
        self.assertEqual(int(match.group("index")), 7)
        self.assertIsNone(LOAD_NAME.search("Loader 007"))

    def test_wake_all_expects_everyone_but_the_speaker_once(self) -> None:
        ledger = Ledger()
        ledger.begin("all")
        ledger.sending("a" * 16, "!room:one", 1, None)
        ledger.sending("b" * 16, "!room:two", 5, 6)
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

    def test_wake_related_only_counts_the_named_agent(self) -> None:
        ledger = Ledger()
        ledger.learn(1, "@one:server")
        ledger.begin("related")
        self.assertEqual(ledger.user_id(1), "@one:server", "认出来的 Matrix 用户 ID 跨段留着")
        ledger.sending("a" * 16, "!room:one", 1, 2)
        ledger.sending("b" * 16, "!room:one", 2, None)
        ledger.received("a" * 16, 3, 1, None)
        ledger.received("b" * 16, 3, 2, None)
        ledger.received("a" * 16, 2, 1, None)
        summary = ledger.summary({"!room:one": 10})
        self.assertEqual(summary["deliveriesExpected"], 1)
        self.assertEqual(summary["deliveriesReceived"], 1)

    def test_growth_reports_and_resets_per_tier(self) -> None:
        growth = Growth()
        growth.took(1.0)
        growth.took(3.0)
        growth.retried("HTTP 503 network_agent.dependency_unavailable")
        first = growth.summary(50)
        self.assertEqual(first["createRetries"], {"HTTP 503 network_agent.dependency_unavailable": 1})
        self.assertEqual(first["createSeconds"], {"p50": 1.0, "p95": 3.0, "max": 3.0})
        self.assertEqual(growth.summary(100)["createRetries"], {})
        self.assertEqual([create_backoff(attempt) for attempt in range(6)], [1, 2, 4, 8, 16, 16])

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
        self.assertEqual(set(LIMIT_OVERRIDES), variables)
        for name, _, variable, _, maximum in NETWORK_AGENT_LIMIT_FIELDS:
            self.assertLessEqual(int(LIMIT_OVERRIDES[variable]), maximum, name)

    def test_arguments(self) -> None:
        arguments = parse_arguments(["--agents", "2", "4", "--wake", "all"])
        self.assertEqual(arguments.agents, [2, 4])
        self.assertEqual(arguments.wake, ["all"])
        for invalid in (["--agents", "4", "2"], ["--rates", "0"], ["--wake", "mentions"]):
            with self.assertRaises(SystemExit):
                parse_arguments(invalid)


if __name__ == "__main__":
    unittest.main()
