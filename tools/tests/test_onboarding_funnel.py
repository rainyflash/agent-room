"""周报里“新用户走到哪一步”那一节：按注册的那一周数人数，不出任何 ID。"""

from __future__ import annotations

from datetime import UTC, datetime
import unittest

from tools.prodops.capacity_report import build_report
from tools.prodops.onboarding_funnel import report_section

NOW = datetime(2026, 11, 9, 1, 23, tzinfo=UTC)
LOBBY = "!lobby:agentroom.chat"


def millis(text: str) -> str:
    return str(int(datetime.fromisoformat(text).replace(tzinfo=UTC).timestamp() * 1000))


class FakeDatabases:
    """按库名和语句里的表名给结果，像生产上那三个库。"""

    def __init__(self) -> None:
        # 10-19 那周三个人：走到底的、只在公共大厅说话的、没验证的。
        self.accounts = [
            ["kc-a", millis("2026-10-20T08:00:00"), "t"],
            ["kc-b", millis("2026-10-21T08:00:00"), "t"],
            ["kc-c", millis("2026-10-22T08:00:00"), "f"],
            # 11-02 那周一个：注册还没满 7 天，“一周后还来”算不出来。
            ["kc-d", millis("2026-11-08T08:00:00"), "t"],
        ]
        # 10-26 那周五个人，四个验证了：看百分比。
        self.accounts += [[f"kc-w{index}", millis("2026-10-27T08:00:00"), "t" if index else "f"] for index in range(5)]
        self.principals = [
            ["kc-a", "@a:agentroom.chat", "t", "2026-11-01 10:00:00+00", "2026-10-25 09:00:00.25+00"],
            ["kc-b", "@b:agentroom.chat", "f", "2026-10-21 08:30:00+00", ""],
            ["kc-d", "@d:agentroom.chat", "f", "", ""],
        ]
        self.agents = [["@agent-x:agentroom.chat"], ["@agent-y:agentroom.chat"]]
        self.memberships = [
            ["!r1:agentroom.chat", "@a:agentroom.chat"],
            ["!r1:agentroom.chat", "@agent-x:agentroom.chat"],
            [LOBBY, "@b:agentroom.chat"],
            [LOBBY, "@agent-x:agentroom.chat"],
            ["!r2:agentroom.chat", "@d:agentroom.chat"],
            ["!r2:agentroom.chat", "@agent-y:agentroom.chat"],
        ]
        self.messages = [
            ["!r1:agentroom.chat", "@agent-x:agentroom.chat", millis("2026-10-20T09:00:00")],
            ["!r1:agentroom.chat", "@a:agentroom.chat", millis("2026-10-21T09:00:00")],
            [LOBBY, "@agent-x:agentroom.chat", millis("2026-10-21T09:00:00")],
            [LOBBY, "@b:agentroom.chat", millis("2026-10-21T09:10:00")],
        ]
        self.statements: list[tuple[str, str]] = []

    def rows(self, database: str, statement: str) -> list[list[str]]:
        self.statements.append((database, statement))
        if database == "keycloak":
            return self.accounts
        if database == "synapse":
            return self.memberships if "room_memberships" in statement else self.messages
        if "agent_room.principal" in statement:
            return self.principals
        if "room_catalog_entry" in statement:
            return [[LOBBY]]
        return self.agents


class OnboardingFunnelTests(unittest.TestCase):
    def test_counts_each_week_without_naming_anyone(self) -> None:
        databases = FakeDatabases()

        section = "\n".join(report_section(databases, NOW))

        self.assertIn("## 新用户走到哪一步", section)
        # 注册 | 验证 | 进应用 | 桌面端 | Agent 进房间 | Agent 说话 | 自己说话 | 一周后还来
        self.assertIn("| 10-19 起 | 3 | 2 | 2 | 1 | 1 | 1 | 2 | 1 |", section)
        self.assertIn("| 10-26 起 | 5 | 4（80%） | 0 | 0 | 0 | 0 | 0 | 0 |", section)
        self.assertIn("| 11-02 起 | 1 | 1 | 1 | 0 | 1 | 0 | 0 | — |", section)
        self.assertIn("| 10-05 起 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |", section)
        self.assertNotIn("11-09 起", section.split("| 10-05 起")[0])
        for secret in ("kc-", "@a:", "@agent-x", "!r1", "agentroom.chat"):
            self.assertNotIn(secret, section)
        keycloak = next(statement for database, statement in databases.statements if database == "keycloak")
        self.assertIn("r.name = 'agent-room'", keycloak)
        self.assertIn(f">= {millis('2026-10-05T00:00:00')}", keycloak)

    def test_only_reads_the_three_databases(self) -> None:
        databases = FakeDatabases()
        report_section(databases, NOW)
        self.assertEqual({database for database, _ in databases.statements}, {"keycloak", "agent_room", "synapse"})
        for _, statement in databases.statements:
            self.assertRegex(statement.strip(), r"^SELECT\b")
            # 只看谁在什么时候发过，不读内容。
            self.assertNotIn("event_json", statement)
            self.assertNotIn("content", statement)

    def test_no_signups_still_draws_the_table(self) -> None:
        databases = FakeDatabases()
        databases.accounts = []
        section = report_section(databases, NOW)
        self.assertIn("| 11-09 起 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |", section)
        self.assertEqual([database for database, _ in databases.statements], ["keycloak"])

    def test_unreadable_database_only_costs_this_section(self) -> None:
        class Broken:
            def rows(self, database: str, statement: str) -> list[list[str]]:
                raise RuntimeError("psql failed for kc-a")

        section = "\n".join(report_section(Broken(), NOW))
        self.assertIn("**这周读不到（RuntimeError）。**", section)
        self.assertNotIn("kc-a", section)

    def test_weekly_report_carries_the_section_even_without_prometheus(self) -> None:
        class Unreachable:
            def query(self, expression: str, at: datetime) -> list[object]:
                from tools.prodops.capacity_report import CapacityReportError

                raise CapacityReportError("连不上")

            def query_range(self, *arguments: object) -> list[object]:
                raise AssertionError("不该走到这里")

        onboarding = report_section(FakeDatabases(), NOW)
        report = "\n".join(
            build_report(Unreachable(), now=NOW, network=None, previous_network=None, version="v", onboarding=onboarding)
        )
        self.assertIn("这周只有新用户和网络流量", report)
        self.assertIn("## 新用户走到哪一步", report)


if __name__ == "__main__":
    unittest.main()
