from __future__ import annotations

import unittest

from tools.prodops.config import NetworkAgentLimits
from tools.prodops.launch_status import (
    GIB,
    MIB,
    ContainerUsage,
    EmailCounts,
    HostUsage,
    LaunchSnapshot,
    LaunchStatusError,
    Lobby,
    NetworkAgentCounts,
    Registrations,
    parse_available_memory,
    parse_container_usage,
    parse_email_counts,
    parse_network_agents,
    parse_registrations,
    report,
)

# 2026-10-15 在生产上读回来的样子。
CONTROL_OUTPUT = (
    '{"lobbies" : [{"slug" : "agent-room-global", "shards" : 1, "slots" : 40, "fullestShard" : 40, '
    '"softCapacity" : 180, "sentThisMinute" : 0}], "inLobbies" : 0, "active" : 0, '
    '"createdHour" : 0, "createdDay" : 1}\n'
)
IDENTITY_OUTPUT = '{"hour" : 3, "today" : 17, "unverifiedToday" : 4}\n'
# Keycloak（Micrometer）的写法：标签最后带一个逗号。
METRICS_OUTPUT = """HTTP/1.0 200 OK
content-type: text/plain

# TYPE keycloak_user_events_total counter
keycloak_user_events_total{client_id="agent-room-web",error="",event="register",idp="",realm="agent-room",} 9.0
keycloak_user_events_total{client_id="agent-room-web",error="",event="send_verify_email",idp="",realm="agent-room",} 9.0
keycloak_user_events_total{client_id="agent-room-web",error="",event="SEND_RESET_PASSWORD",idp="",realm="agent-room",} 2.0
keycloak_user_events_total{client_id="agent-room-web",error="email_send_failed",event="send_verify_email_error",idp="",realm="agent-room",} 1.0
jvm_threads_live_threads 42.0
"""
STATS_OUTPUT = (
    "agent-room-synapse-1\t97.31%\t1.1GiB / 3.824GiB\n"
    "agent-room-postgres-1\t12.00%\t265.6MiB / 3.824GiB\n"
    "someone-else-1\t50.00%\t1GiB / 3.824GiB\n"
    "agent-room-object-store-1\t--\t0B / 0B\n"
)


def quiet_snapshot() -> LaunchSnapshot:
    return LaunchSnapshot(
        network_agents=parse_network_agents(CONTROL_OUTPUT),
        registrations=Registrations(hour=0, today=0, unverified_today=0),
        emails=None,
        host=HostUsage(
            containers=(ContainerUsage("agent-room-synapse-1", 1.7, "156.8MiB / 3.824GiB"),),
            disk_free_bytes=41 * GIB,
            memory_available_bytes=1400 * MIB,
        ),
    )


class LaunchStatusTests(unittest.TestCase):
    def test_reads_what_the_two_queries_return(self) -> None:
        counts = parse_network_agents(CONTROL_OUTPUT)
        self.assertEqual(
            counts.lobbies,
            (Lobby("agent-room-global", shards=1, slots=40, fullest_shard=40, soft_capacity=180, sent_this_minute=0),),
        )
        self.assertEqual((counts.in_lobbies, counts.active, counts.created_hour, counts.created_day), (0, 0, 0, 1))
        self.assertEqual(parse_registrations(IDENTITY_OUTPUT), Registrations(hour=3, today=17, unverified_today=4))
        for broken in ("", "not json\n", "[1]\n", CONTROL_OUTPUT * 2):
            with self.subTest(broken=broken), self.assertRaises(LaunchStatusError):
                parse_registrations(broken)

    def test_counts_sent_and_failed_emails_from_the_identity_metrics(self) -> None:
        self.assertEqual(parse_email_counts(METRICS_OUTPUT), EmailCounts(sent=11, failed=1))
        # 这版还没打开用户事件指标，或者启动以来一封都没发过。
        self.assertIsNone(parse_email_counts("jvm_threads_live_threads 42.0\n"))
        self.assertIsNone(parse_email_counts(""))

    def test_keeps_only_this_deployments_containers(self) -> None:
        usage = parse_container_usage(STATS_OUTPUT, "agent-room")
        self.assertEqual([container.name for container in usage], ["agent-room-synapse-1", "agent-room-postgres-1"])
        self.assertAlmostEqual(usage[0].cpu_percent, 97.31)
        self.assertEqual(parse_available_memory("MemTotal: 4009000 kB\nMemAvailable:  1433600 kB\n"), 1400 * MIB)
        self.assertIsNone(parse_available_memory("MemTotal: 4009000 kB\n"))

    def test_a_quiet_day_has_nothing_to_flag(self) -> None:
        lines = report(quiet_snapshot(), NetworkAgentLimits())

        self.assertEqual(lines[0], "公开大厅里的网络 Agent：0 / 200")
        self.assertIn("这一分钟网络 Agent 合起来说了 0 / 12 条", lines[1])
        self.assertIn("发信：身份服务这次启动以来还没有发信的记录", lines)
        self.assertIn("磁盘：还剩 41.0 GiB", lines)
        self.assertEqual(lines[-1], "没有要注意的。")

    def test_flags_each_thing_the_runbook_acts_on(self) -> None:
        busy_lobby = Lobby("agent-room-global", shards=2, slots=300, fullest_shard=180, soft_capacity=180,
                           sent_this_minute=12)
        snapshot = LaunchSnapshot(
            network_agents=NetworkAgentCounts(lobbies=(busy_lobby,), in_lobbies=180, active=150, created_hour=40,
                                              created_day=200),
            registrations=Registrations(hour=30, today=80, unverified_today=20),
            emails=EmailCounts(sent=79, failed=2),
            host=HostUsage(
                containers=parse_container_usage(STATS_OUTPUT, "agent-room"),
                disk_free_bytes=19 * GIB,
                memory_available_bytes=200 * MIB,
            ),
        )

        lines = report(snapshot, NetworkAgentLimits())
        warnings = lines[lines.index("注意：") + 1 :]

        self.assertEqual(len(warnings), 7, warnings)
        self.assertIn("快满了（180 / 200）", warnings[0])
        self.assertIn("agent-room-global 这一分钟网络 Agent 合起来的 12 条用完了", warnings[1])
        self.assertIn("今天已经注册了 80 个账户", warnings[2])
        self.assertIn("有 2 封信没发出去", warnings[3])
        self.assertIn("agent-room-synapse-1 的 CPU 到了 97%", warnings[4])
        self.assertIn("内存只剩 200 MiB", warnings[5])
        self.assertIn("磁盘只剩 19.0 GiB", warnings[6])
        # 调过的上限照调过的算。
        relaxed = report(snapshot, NetworkAgentLimits(max_lobby_agents=500, lobby_total_messages_per_minute=30))
        self.assertFalse(any("快满了" in line or "条用完了" in line for line in relaxed))


if __name__ == "__main__":
    unittest.main()
