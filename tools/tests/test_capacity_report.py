from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
import tempfile
import unittest

from tools.prodops.capacity_report import (
    BUSIEST,
    LABEL,
    QUERIES,
    STORAGE,
    CapacityReportError,
    NetworkReading,
    ReportState,
    Sample,
    build_report,
    latency_query,
    post_report,
    read_network,
    week_label,
)

NOW = datetime(2026, 11, 2, 1, 23, tzinfo=UTC)
GIB = 1024**3


class FakePrometheus:
    """按表达式给结果；一周前的存储另给一份。没写的表达式回空，像刚开监控时那样。"""

    def __init__(
        self,
        values: dict[str, list[Sample]] | None = None,
        before: dict[str, list[Sample]] | None = None,
        ranges: dict[str, list[tuple[datetime, float]]] | None = None,
    ) -> None:
        self.values = values or {}
        self.before = before or {}
        self.ranges = ranges or {}

    def query(self, expression: str, at: datetime) -> list[Sample]:
        table = self.before if at < NOW else self.values
        return table.get(expression, [])

    def query_range(
        self, expression: str, start: datetime, end: datetime, step: timedelta
    ) -> list[tuple[datetime, float]]:
        return self.ranges.get(expression, [])


class Unreachable:
    def query(self, expression: str, at: datetime) -> list[Sample]:
        raise CapacityReportError("URLError")

    def query_range(
        self, expression: str, start: datetime, end: datetime, step: timedelta
    ) -> list[tuple[datetime, float]]:
        raise CapacityReportError("URLError")


def one(value: float, **labels: str) -> list[Sample]:
    return [Sample(labels, value)]


def busy_week() -> FakePrometheus:
    values = {
        QUERIES["accounts"]: [
            Sample({"region": "cn", "kind": "person"}, 12),
            Sample({"region": "cn", "kind": "local_agent"}, 30),
            Sample({"region": "europe", "kind": "person"}, 5),
            Sample({"region": "europe", "kind": "network_agent"}, 9),
        ],
        QUERIES["requests"]: [Sample({"region": "cn", "kind": "person"}, 15234)],
        QUERIES["samples"]: [Sample({"region": "cn"}, 410), Sample({"region": "europe"}, 88)],
        latency_query("api_round_trip", 0.5): [Sample({"region": "cn"}, 0.21)],
        latency_query("api_round_trip", 0.95): [Sample({"region": "cn"}, 0.84)],
        latency_query("largest_contentful_paint", 0.95): [Sample({"region": "europe"}, 1.9)],
        QUERIES["synapse_cpu"]: one(0.82),
        QUERIES["host_cpu"]: one(0.37),
        QUERIES["cores"]: one(4),
        QUERIES["memory_used"]: one(6.2 * GIB),
        QUERIES["memory_available_min"]: one(300 * 1024**2),
        QUERIES["memory_total"]: one(7.8 * GIB),
        QUERIES["events_per_minute"]: one(42),
        QUERIES["lobby_slots"]: one(37),
        QUERIES["lobby_agents"]: one(21),
        QUERIES["emails"]: one(64),
        QUERIES["alerts"]: [
            Sample({"alertname": "AgentRoomApiLatencyHigh", "severity": "warning"}, 35),
            Sample({"alertname": "AgentRoomCoreEndpointUnavailable", "severity": "critical"}, 4),
        ],
        STORAGE["databases"]: [
            Sample({"datname": "agent_room"}, 2.0 * GIB),
            Sample({"datname": "synapse"}, 5.0 * GIB),
            Sample({"datname": "keycloak"}, 0.1 * GIB),
        ],
        STORAGE["objects"]: one(3 * GIB),
        STORAGE["backups"]: one(14 * GIB),
        STORAGE["prometheus"]: one(0.5 * GIB),
        STORAGE["disk_used"]: one(50 * GIB),
        STORAGE["disk_free"]: one(30 * GIB),
    }
    before = {
        STORAGE["databases"]: [Sample({"datname": "synapse"}, 4.5 * GIB)],
        STORAGE["disk_used"]: one(48 * GIB),
    }
    hours = [NOW - timedelta(hours=hour) for hour in range(168, 0, -1)]
    ranges = {
        BUSIEST["Synapse 存下的事件"]: [(moment, 100.0) for moment in hours[:-1]] + [(hours[-1], 2600.0)],
    }
    return FakePrometheus(values, before, ranges)


class ReportTests(unittest.TestCase):
    def test_busy_week_reads_like_a_report(self) -> None:
        report = "\n".join(
            build_report(busy_week(), now=NOW, network=None, previous_network=None, version="0.1.0-alpha.86 (abc1234)")
        )

        self.assertIn("# 容量周报 2026-10-26 – 2026-11-02", report)
        self.assertIn("| 中国大陆 | 12 | 30 | – | 15,234 / – / – |", report)
        self.assertIn("| 欧洲 | 5 | – | 9 |", report)
        self.assertIn("| 中国大陆 | 210 / 840 | – / – | 410 |", report)
        self.assertIn("| 欧洲 | – / – | – / 1,900 | 88 |", report)
        self.assertIn("Synapse 的 CPU，P95：0.82 个核", report)
        self.assertIn("整机 CPU，P95：37%（4 个核）", report)
        self.assertIn("| Synapse 的库 | 5.0 GiB | 4.5 GiB | +512.0 MiB |", report)
        # 盘：30 GiB 剩、每周涨 2 GiB，离 20 GiB 还有 5 周。
        self.assertIn("约 5 周后低于部署预检要的 20 GiB", report)
        self.assertIn("Synapse 存下的事件最忙的一小时：2,600 个，到 11-02 00:23 UTC 为止", report)
        self.assertIn("| AgentRoomApiLatencyHigh | warning | 约 35 分钟 |", report)
        # 三条起点都过了。
        self.assertIn("超过了 0.7 个核", report)
        self.assertIn("低于 500 MB", report)
        self.assertIn("60 天内就不够", report)
        # 只出中文的地区名，不出标签的原样。
        for raw in ("cn", "europe", "local_agent", "network_agent"):
            self.assertNotIn(f"| {raw} ", report)

    def test_fresh_monitoring_says_there_is_nothing_yet(self) -> None:
        report = "\n".join(build_report(FakePrometheus(), now=NOW, network=None, previous_network=None, version="v"))

        self.assertIn("还没有按地区的数", report)
        self.assertIn("Synapse 的 CPU，P95：还没有数", report)
        self.assertIn("盘还剩多少：还没有数。", report)
        self.assertIn("最忙的一小时：还没有数", report)
        self.assertIn("## 这周响过的告警\n\n没有。", report)
        self.assertIn("都还在设计里定的起点以下。", report)

    def test_unreachable_prometheus_still_reports_the_network(self) -> None:
        previous = NetworkReading(NOW - timedelta(days=7), "boot", 0, 0)
        current = NetworkReading(NOW, "boot", 3 * GIB, 40 * GIB)

        report = "\n".join(build_report(Unreachable(), now=NOW, network=current, previous_network=previous, version="v"))

        self.assertIn("Prometheus 读不到", report)
        self.assertIn("进 3.0 GiB，出 40.0 GiB", report)
        self.assertNotIn("## 机器", report)

    def test_network_needs_a_previous_reading_from_the_same_boot(self) -> None:
        current = NetworkReading(NOW, "boot-2", 10, 10)
        first = "\n".join(build_report(Unreachable(), now=NOW, network=current, previous_network=None, version="v"))
        rebooted = "\n".join(
            build_report(
                Unreachable(),
                now=NOW,
                network=current,
                previous_network=NetworkReading(NOW - timedelta(days=7), "boot-1", 5, 5),
                version="v",
            )
        )

        self.assertIn("第一次记，下一份报告才有一周的数。", first)
        self.assertIn("服务器重启过", rebooted)


class NetworkTests(unittest.TestCase):
    def test_counts_only_the_host_interfaces(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            (proc / "net").mkdir()
            (proc / "sys" / "kernel" / "random").mkdir(parents=True)
            (proc / "sys" / "kernel" / "random" / "boot_id").write_text("boot-1\n", encoding="utf-8")
            (proc / "net" / "dev").write_text(
                "Inter-|   Receive                                                |  Transmit\n"
                " face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets\n"
                "    lo: 900 9 0 0 0 0 0 0 900 9 0 0 0 0 0 0\n"
                "  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n"
                "docker0: 50 1 0 0 0 0 0 0 60 1 0 0 0 0 0 0\n"
                "br-1a2b: 50 1 0 0 0 0 0 0 60 1 0 0 0 0 0 0\n"
                "veth9f: 50 1 0 0 0 0 0 0 60 1 0 0 0 0 0 0\n",
                encoding="utf-8",
            )

            reading = read_network(NOW, proc)

        self.assertEqual(reading, NetworkReading(NOW, "boot-1", 1000, 2000))

    def test_missing_proc_is_no_reading(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertIsNone(read_network(NOW, Path(directory)))


class FakeTracker:
    def __init__(self) -> None:
        self.created: list[tuple[str, str, list[str]]] = []
        self.closed: list[tuple[int, str]] = []

    def create(self, title: str, body: str, labels: list[str]) -> int:
        self.created.append((title, body, labels))
        return 40 + len(self.created)

    def close(self, number: int, comment: str) -> None:
        self.closed.append((number, comment))


class PostTests(unittest.TestCase):
    def test_posts_once_a_week_and_closes_the_previous_report(self) -> None:
        tracker = FakeTracker()
        state = ReportState(last_week="2026-W43", last_issue=40)

        number = post_report(tracker, state, ["# 报告"], now=NOW)

        self.assertEqual(number, 41)
        title, body, labels = tracker.created[0]
        self.assertEqual(title, "容量周报 2026-10-26 – 2026-11-02")
        self.assertEqual(labels, [LABEL])
        self.assertNotIn("prod-error", labels)
        self.assertIn("<!-- agent-room-capacity-report: 2026-W44 -->", body)
        self.assertEqual(tracker.closed, [(40, "下一份是 #41。")])
        self.assertEqual((state.last_week, state.last_issue), ("2026-W44", 41))

        self.assertIsNone(post_report(tracker, state, ["# 报告"], now=NOW + timedelta(hours=1)))
        self.assertEqual(len(tracker.created), 1)

    def test_state_round_trips(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capacity-report" / "state.json"
            state = ReportState(NetworkReading(NOW, "boot-1", 1, 2), "2026-W44", 41)
            state.save(path)

            loaded = ReportState.load(path)

        self.assertEqual(loaded, state)

    def test_week_is_the_week_the_seven_days_start_in(self) -> None:
        self.assertEqual(week_label(NOW), "2026-W44")
        self.assertEqual(week_label(datetime(2027, 1, 4, 1, 23, tzinfo=UTC)), "2026-W53")


if __name__ == "__main__":
    unittest.main()
