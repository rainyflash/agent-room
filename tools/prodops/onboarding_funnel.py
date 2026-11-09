"""新用户走到哪一步：容量周报的一节（设计在 `specs/onboarding-funnel/design.md` 第 1 节）。

按注册的那一周分组，从身份服务、控制面和聊天服务器现有的数据里数，每一周注册的人各走到了哪一步。
每个人的记录只在这里对一下，出去的只有人数：不写进任何地方，不读消息内容（只看谁在什么时候、在哪个
房间发过消息）。
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
import re
from typing import Protocol

WEEKS = 6
COMEBACK_AFTER = timedelta(days=7)
# 人数少于这个的那一周不写百分比，免得一两个人把比例拉得很难看。
PERCENT_FLOOR = 5
REALM = "agent-room"
NETWORK_AGENT_ISSUER = "urn:agent-room:network-agent"
# 网页登录连续 30 天不用才过期，用着就续、最长一年（specs/session-lifetime/design.md）。续的时候写的是
# 新的到期时间，往回推 30 天就是最后一次用的时间；到了一年上限的那种推不出来，不算。
WEB_SESSION_IDLE = "30 days"
WEB_SESSION_MAXIMUM = "365 days"
# 聊天消息：私人房间里是加密事件，公共大厅和不加密的公开房间里是我们自己的消息事件（含改过的）。
MESSAGE_TYPES = "type = 'm.room.encrypted' OR type LIKE 'io.github.rainyflash.agentroom.message.%'"

STEPS = (
    ("registered", "注册"),
    ("verified", "验证邮箱"),
    ("in_app", "进过应用"),
    ("computer", "装了桌面端"),
    ("agent_joined", "Agent 进了房间"),
    ("agent_spoke", "Agent 说过话"),
    ("spoke", "自己说过话"),
    ("came_back", "一周后还来"),
)


class Databases(Protocol):
    def rows(self, database: str, statement: str) -> list[list[str]]: ...


class RuntimeDatabases:
    """用部署运行时在 PostgreSQL 容器里跑只读的 psql；一行一条，字段用 `|` 隔开。"""

    def __init__(self, runtime: object) -> None:
        self._runtime = runtime

    def rows(self, database: str, statement: str) -> list[list[str]]:
        output = self._runtime._embedded_psql(database, statement)  # type: ignore[attr-defined]  # noqa: SLF001
        return [line.split("|") for line in output.splitlines() if line.strip()]


@dataclass
class Person:
    registered_at: datetime
    verified: bool
    in_app: bool = False
    computer: bool = False
    agent_joined: bool = False
    agent_spoke: bool = False
    spoke: bool = False
    last_used: datetime | None = None

    @property
    def registered(self) -> bool:
        return True

    def came_back(self, now: datetime) -> bool | None:
        """注册满 7 天的才算得出来；没满的给 None。"""
        if now - self.registered_at < COMEBACK_AFTER:
            return None
        return self.last_used is not None and self.last_used >= self.registered_at + COMEBACK_AFTER


def _moment(value: str) -> datetime | None:
    value = value.strip()
    if not value:
        return None
    if value.isdigit():
        return datetime.fromtimestamp(int(value) / 1000, UTC)
    # psql 写的是 `2026-11-08 12:20:01.5+00`：时区只有小时，补上分钟再交给 fromisoformat。
    if re.search(r"[+-]\d\d$", value):
        value += ":00"
    parsed = datetime.fromisoformat(value.replace(" ", "T"))
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=UTC)


def _later(*moments: datetime | None) -> datetime | None:
    present = [moment for moment in moments if moment is not None]
    return max(present) if present else None


def week_start(moment: datetime) -> datetime:
    day = moment.astimezone(UTC).replace(hour=0, minute=0, second=0, microsecond=0)
    return day - timedelta(days=day.weekday())


def collect(databases: Databases, now: datetime) -> list[Person]:
    """最近几周注册的人，各走到了哪一步。"""
    since = week_start(now) - timedelta(weeks=WEEKS - 1)
    since_ms = int(since.timestamp() * 1000)
    accounts = {
        subject: Person(registered_at=_moment(created) or since, verified=verified == "t")
        for subject, created, verified in databases.rows(
            "keycloak",
            f"""SELECT u.id, u.created_timestamp, u.email_verified
                FROM user_entity u JOIN realm r ON r.id = u.realm_id
                WHERE r.name = '{REALM}' AND u.service_account_client_link IS NULL
                  AND u.created_timestamp >= {since_ms}""",
        )
    }
    if not accounts:
        return []
    people: dict[str, Person] = {}
    for subject, matrix_user, computer, web_used, computer_used in databases.rows(
        "agent_room",
        f"""SELECT p.oidc_subject, coalesce(p.matrix_user_id, ''),
                   EXISTS (SELECT 1 FROM agent_room.device d
                           WHERE d.principal_id = p.id AND d.platform <> 'network'),
                   (SELECT max(least(w.expires_at - interval '{WEB_SESSION_IDLE}',
                                     coalesce(w.revoked_at, 'infinity'::timestamptz)))
                    FROM agent_room.web_session w
                    WHERE w.principal_id = p.id
                      AND w.expires_at < w.authenticated_at + interval '{WEB_SESSION_MAXIMUM}'),
                   (SELECT max(d.last_seen_at) FROM agent_room.device d
                    WHERE d.principal_id = p.id AND d.platform <> 'network')
            FROM agent_room.principal p
            WHERE p.oidc_issuer <> '{NETWORK_AGENT_ISSUER}'""",
    ):
        person = accounts.get(subject)
        if person is None:
            continue
        person.in_app = True
        person.computer = computer == "t"
        person.last_used = _later(_moment(web_used), _moment(computer_used))
        if matrix_user:
            people[matrix_user] = person
    if people:
        _chat(databases, people)
    return list(accounts.values())


def _chat(databases: Databases, people: dict[str, Person]) -> None:
    """谁在哪些房间（不算公共大厅）、房间里有没有 Agent 进来和说话、他自己最后一次说话是什么时候。"""
    agents = {
        row[0]
        for row in databases.rows(
            "agent_room", "SELECT matrix_user_id FROM agent_room.agent WHERE matrix_user_id IS NOT NULL"
        )
    }
    lobbies = {
        row[0]
        for row in databases.rows(
            "agent_room",
            """SELECT i.matrix_room_id FROM agent_room.room_instance i
               JOIN agent_room.room_catalog_entry e ON e.id = i.catalog_entry_id
               WHERE e.kind = 'public_lobby' AND i.matrix_room_id IS NOT NULL""",
        )
    }
    members: dict[str, set[str]] = {}
    for room, user in databases.rows(
        "synapse", "SELECT DISTINCT room_id, user_id FROM room_memberships WHERE membership = 'join'"
    ):
        if room not in lobbies:
            members.setdefault(room, set()).add(user)
    speakers: dict[str, set[str]] = {}
    for room, sender, last_sent in databases.rows(
        "synapse",
        f"SELECT room_id, sender, max(origin_server_ts) FROM events WHERE {MESSAGE_TYPES} GROUP BY room_id, sender",
    ):
        speakers.setdefault(room, set()).add(sender)
        person = people.get(sender)
        if person is not None:
            person.spoke = True
            person.last_used = _later(person.last_used, _moment(last_sent))
    for room, users in members.items():
        joined_agents = users & agents
        if not joined_agents:
            continue
        spoke = bool(speakers.get(room, set()) & agents)
        for user in users:
            person = people.get(user)
            if person is not None:
                person.agent_joined = True
                person.agent_spoke = person.agent_spoke or spoke


def _cell(count: int, total: int) -> str:
    if total < PERCENT_FLOOR or count == 0:
        return str(count)
    return f"{count}（{round(count * 100 / total)}%）"


def section(people: Sequence[Person], now: datetime) -> list[str]:
    """给人看的一节：最近几周每一周一行，只有人数。"""
    lines = [
        "## 新用户走到哪一步",
        "",
        "按注册的那一周分组，从身份服务、控制面和聊天服务器现有的数据里数，只有人数"
        "（设计在 `specs/onboarding-funnel/design.md`）。",
        "",
        "| 注册那周 | " + " | ".join(label for _, label in STEPS) + " |",
        "| --- |" + " ---: |" * len(STEPS),
    ]
    first = week_start(now) - timedelta(weeks=WEEKS - 1)
    for index in range(WEEKS):
        start = first + timedelta(weeks=index)
        cohort = [person for person in people if start <= person.registered_at < start + timedelta(weeks=1)]
        total = len(cohort)
        cells = []
        for key, _ in STEPS:
            if key == "came_back":
                answers = [person.came_back(now) for person in cohort]
                if total and all(answer is None for answer in answers):
                    cells.append("—")
                    continue
                count = sum(1 for answer in answers if answer)
            else:
                count = sum(1 for person in cohort if getattr(person, key))
            cells.append(str(count) if key == "registered" else _cell(count, total))
        lines.append(f"| {start:%m-%d} 起 | " + " | ".join(cells) + " |")
    lines.extend(
        [
            "",
            "- “Agent 进了房间”“Agent 说过话”看他在的房间（不算公共大厅），本机的和网络的 Agent 都算，别人带进来的也算。",
            "- “一周后还来”：注册 7 天以后发过消息、网页登录用过或者电脑用过；注册还没满 7 天的写“—”，"
            "上一周注册的人能“还来”的时间还短，这个数会偏小，下周再看。",
            "- 维护者自己的账户和发版验收用的都算在里面；人数少于 5 的那一周不写百分比。",
            "",
        ]
    )
    return lines


def report_section(databases: Databases, now: datetime) -> list[str]:
    """读不到就照实说一句，不让整份周报失败。"""
    try:
        people = collect(databases, now)
    except Exception as error:  # noqa: BLE001 - 哪个库读不到都只影响这一节
        return ["## 新用户走到哪一步", "", f"**这周读不到（{type(error).__name__}）。**", ""]
    return section(people, now)
