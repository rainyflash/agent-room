"""首发当天看数（`production.py launch-status`）。

生产上没开监控（`telemetry.enabled` 是 false，没有 Prometheus、Grafana，也没有告警会响），首发那几天
要盯的几样由这一条命令直接读出来：公开大厅的分片和网络 Agent、今天注册了多少、发信、容器的 CPU、
内存和磁盘。全是只读的：两条查询只有 SELECT，身份服务的指标、`docker stats`、磁盘和内存都只是读。
"""

from __future__ import annotations

from dataclasses import dataclass
import json
import math
import re

from .config import NetworkAgentLimits

GIB = 1024**3
MIB = 1024**2
# 发版部署的预检要这么多可用空间（`runtime.MINIMUM_DISK_BYTES`）。
DEPLOY_DISK_FLOOR_BYTES = 20 * GIB
# Resend 免费档每天 100 封，UTC 零点重置；每个新注册的账户发一封验证信。
EMAIL_DAILY_QUOTA = 100
EMAIL_WARNING = 80
LOBBY_WARNING_RATIO = 0.9
# Synapse 是一个进程，最多用满一个核。
SYNAPSE_BUSY_PERCENT = 90.0
MEMORY_WARNING_BYTES = 300 * MIB

# 和运行指标（`operational_metrics.rs`）同一套算法：只数活着的分片、没停用的网络 Agent。
# 大厅总条数的桶是第 11 步的 `lobby-send:minute:<大厅 ID 去掉横线>`，到了上限就不再加。
CONTROL_QUERY = r"""
SELECT json_build_object(
  'lobbies', COALESCE((
    SELECT json_agg(json_build_object(
             'slug', lobby.slug,
             'shards', lobby.shards,
             'slots', lobby.slots,
             'fullestShard', lobby.fullest_shard,
             'softCapacity', lobby.soft_capacity,
             'sentThisMinute', lobby.sent_this_minute)
           ORDER BY lobby.slug)
      FROM (
        SELECT catalog.slug,
               count(instance.id) AS shards,
               COALESCE(sum(instance.allocated_slots), 0) AS slots,
               COALESCE(max(instance.allocated_slots), 0) AS fullest_shard,
               COALESCE(min(instance.soft_capacity), 0) AS soft_capacity,
               COALESCE((
                 SELECT window_row.event_count
                   FROM agent_room.network_agent_rate_window window_row
                  WHERE window_row.bucket = 'lobby-send:minute:' || replace(catalog.id::text, '-', '')
                    AND window_row.window_started_at > now() - interval '1 minute'), 0) AS sent_this_minute
          FROM agent_room.room_catalog_entry catalog
          LEFT JOIN agent_room.room_instance instance
            ON instance.catalog_entry_id = catalog.id AND instance.state = 'active'
         WHERE catalog.kind = 'public_lobby'
         GROUP BY catalog.id, catalog.slug) lobby), '[]'::json),
  'inLobbies', (SELECT count(DISTINCT agent.id)
      FROM agent_room.network_agent agent
      JOIN agent_room.network_agent_room room ON room.network_agent_id = agent.id
      JOIN agent_room.room_catalog_entry catalog ON catalog.id = room.catalog_entry_id
     WHERE agent.status <> 'disabled' AND catalog.kind = 'public_lobby'),
  'active', (SELECT count(*) FROM agent_room.network_agent
     WHERE status = 'active' AND last_active_at > now() - interval '5 minutes'),
  'createdHour', (SELECT count(*) FROM agent_room.network_agent
     WHERE created_at > now() - interval '1 hour'),
  'createdDay', (SELECT count(*) FROM agent_room.network_agent
     WHERE created_at > now() - interval '24 hours')
)
"""

# Keycloak 的库：`created_timestamp` 是毫秒。不算服务账号（控制面删账户用的那个）。
IDENTITY_QUERY = r"""
WITH bounds AS (
  SELECT (extract(epoch FROM now() - interval '1 hour') * 1000)::bigint AS hour_ago,
         (extract(epoch FROM date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC') * 1000)::bigint
           AS utc_midnight
)
SELECT json_build_object(
  'hour', count(*) FILTER (WHERE users.created_timestamp > bounds.hour_ago),
  'today', count(*) FILTER (WHERE users.created_timestamp >= bounds.utc_midnight),
  'unverifiedToday', count(*) FILTER (
    WHERE NOT users.email_verified AND users.created_timestamp >= bounds.utc_midnight)
)
  FROM bounds
  JOIN realm ON realm.name = 'agent-room'
  JOIN user_entity users ON users.realm_id = realm.id
 WHERE users.service_account_client_link IS NULL
"""

# 和告警规则里的发信事件一样（大小写不分，带不带 `_error` 都算）。
EMAIL_EVENT = re.compile(r"send_(verify_email|reset_password)(_error)?", re.IGNORECASE)
USER_EVENT_LINE = re.compile(r"^keycloak_user_events_total\{(?P<labels>[^}]*)\}\s+(?P<value>\S+)")
METRIC_LABEL = re.compile(r'(\w+)="((?:[^"\\]|\\.)*)"')


@dataclass(frozen=True, slots=True)
class Lobby:
    slug: str
    shards: int
    slots: int
    fullest_shard: int
    soft_capacity: int
    sent_this_minute: int


@dataclass(frozen=True, slots=True)
class NetworkAgentCounts:
    lobbies: tuple[Lobby, ...]
    in_lobbies: int
    active: int
    created_hour: int
    created_day: int


@dataclass(frozen=True, slots=True)
class Registrations:
    hour: int
    today: int
    unverified_today: int


@dataclass(frozen=True, slots=True)
class EmailCounts:
    """身份服务这次启动以来的发信，容器重建就从零算。"""

    sent: int
    failed: int


@dataclass(frozen=True, slots=True)
class ContainerUsage:
    name: str
    cpu_percent: float
    memory: str


@dataclass(frozen=True, slots=True)
class HostUsage:
    containers: tuple[ContainerUsage, ...]
    disk_free_bytes: int
    memory_available_bytes: int | None


@dataclass(frozen=True, slots=True)
class LaunchSnapshot:
    network_agents: NetworkAgentCounts
    registrations: Registrations
    emails: EmailCounts | None
    host: HostUsage


class LaunchStatusError(ValueError):
    """读回来的数不是预期的样子。"""


def parse_network_agents(output: str) -> NetworkAgentCounts:
    value = _single_json(output)
    lobbies = tuple(
        Lobby(
            slug=str(row["slug"]),
            shards=int(row["shards"]),
            slots=int(row["slots"]),
            fullest_shard=int(row["fullestShard"]),
            soft_capacity=int(row["softCapacity"]),
            sent_this_minute=int(row["sentThisMinute"]),
        )
        for row in value["lobbies"]
    )
    return NetworkAgentCounts(
        lobbies=lobbies,
        in_lobbies=int(value["inLobbies"]),
        active=int(value["active"]),
        created_hour=int(value["createdHour"]),
        created_day=int(value["createdDay"]),
    )


def parse_registrations(output: str) -> Registrations:
    value = _single_json(output)
    return Registrations(
        hour=int(value["hour"]),
        today=int(value["today"]),
        unverified_today=int(value["unverifiedToday"]),
    )


def parse_email_counts(metrics: str) -> EmailCounts | None:
    """从身份服务的 `/metrics` 里数发信；一封都没记过（或者这版还没打开用户事件指标）时是 None。"""
    sent = failed = 0.0
    seen = False
    for line in metrics.splitlines():
        match = USER_EVENT_LINE.match(line)
        if match is None:
            continue
        labels = dict(METRIC_LABEL.findall(match["labels"]))
        if not EMAIL_EVENT.fullmatch(labels.get("event", "")):
            continue
        seen = True
        if labels.get("error", ""):
            failed += float(match["value"])
        else:
            sent += float(match["value"])
    return EmailCounts(sent=int(sent), failed=int(failed)) if seen else None


def parse_container_usage(output: str, project: str) -> tuple[ContainerUsage, ...]:
    """`docker stats --no-stream` 每行“名字\\tCPU%\\t内存”；只留这个部署的容器。"""
    usage: list[ContainerUsage] = []
    for line in output.splitlines():
        fields = line.split("\t")
        if len(fields) != 3 or not fields[0].startswith(f"{project}-"):
            continue
        try:
            cpu = float(fields[1].strip().rstrip("%"))
        except ValueError:
            continue
        usage.append(ContainerUsage(name=fields[0], cpu_percent=cpu, memory=fields[2].strip()))
    return tuple(usage)


def parse_available_memory(meminfo: str) -> int | None:
    for line in meminfo.splitlines():
        fields = line.split()
        if len(fields) >= 2 and fields[0] == "MemAvailable:" and fields[1].isdigit():
            return int(fields[1]) * 1024
    return None


def report(snapshot: LaunchSnapshot, limits: NetworkAgentLimits) -> list[str]:
    """给人看的几行，最后是要注意的地方。"""
    lines = [
        *_network_agent_lines(snapshot.network_agents, limits),
        *_identity_lines(snapshot.registrations, snapshot.emails),
        *_host_lines(snapshot.host),
    ]
    warnings = _warnings(snapshot, limits)
    if warnings:
        lines.append("注意：")
        lines.extend(f"- {warning}" for warning in warnings)
    else:
        lines.append("没有要注意的。")
    return lines


def _network_agent_lines(counts: NetworkAgentCounts, limits: NetworkAgentLimits) -> list[str]:
    lines = [f"公开大厅里的网络 Agent：{counts.in_lobbies} / {limits.max_lobby_agents}"]
    for lobby in counts.lobbies:
        lines.append(
            f"  {lobby.slug}：{lobby.shards} 个分片，占着 {lobby.slots} 个位置"
            f"（最满的一个 {lobby.fullest_shard} / {lobby.soft_capacity}，到了开下一个）；"
            f"这一分钟网络 Agent 合起来说了 {lobby.sent_this_minute} / "
            f"{limits.lobby_total_messages_per_minute} 条"
        )
    lines.append(
        f"网络 Agent：最近 5 分钟用过接口的 {counts.active} 个；"
        f"新建的最近一小时 {counts.created_hour} 个、24 小时 {counts.created_day} 个"
    )
    return lines


def _identity_lines(registrations: Registrations, emails: EmailCounts | None) -> list[str]:
    lines = [
        f"注册：今天（UTC）{registrations.today} 个账户，其中还没验证邮箱的 "
        f"{registrations.unverified_today} 个；最近一小时 {registrations.hour} 个"
    ]
    if emails is None:
        lines.append("发信：身份服务这次启动以来还没有发信的记录")
    else:
        lines.append(
            f"发信：身份服务这次启动以来发出 {emails.sent} 封、失败 {emails.failed} 封"
            f"（Resend 免费档每天 {EMAIL_DAILY_QUOTA} 封，UTC 零点重置）"
        )
    return lines


def _host_lines(host: HostUsage) -> list[str]:
    lines = ["容器："]
    lines.extend(
        f"  {container.name}  CPU {container.cpu_percent:.1f}%  内存 {container.memory}"
        for container in host.containers
    )
    if host.memory_available_bytes is not None:
        lines.append(f"内存：还能用 {host.memory_available_bytes / GIB:.1f} GiB")
    lines.append(f"磁盘：还剩 {host.disk_free_bytes / GIB:.1f} GiB")
    return lines


def _warnings(snapshot: LaunchSnapshot, limits: NetworkAgentLimits) -> list[str]:
    counts, host = snapshot.network_agents, snapshot.host
    warnings: list[str] = []
    if counts.in_lobbies >= math.ceil(limits.max_lobby_agents * LOBBY_WARNING_RATIO):
        warnings.append(
            f"公开大厅里的网络 Agent 快满了（{counts.in_lobbies} / {limits.max_lobby_agents}）："
            "满了以后新来的进不来，见预案“大厅满了”"
        )
    warnings.extend(
        f"{lobby.slug} 这一分钟网络 Agent 合起来的 {limits.lobby_total_messages_per_minute} 条用完了，"
        "后说的要等到下一分钟"
        for lobby in counts.lobbies
        if lobby.sent_this_minute >= limits.lobby_total_messages_per_minute
    )
    warnings.extend(_identity_warnings(snapshot.registrations, snapshot.emails))
    warnings.extend(
        f"{container.name} 的 CPU 到了 {container.cpu_percent:.0f}%：Synapse 是一个进程，最多用满一个核，"
        "连着几分钟这样就是顶不住了，见预案“顶不住时按顺序来”"
        for container in host.containers
        if "synapse" in container.name and container.cpu_percent >= SYNAPSE_BUSY_PERCENT
    )
    if host.memory_available_bytes is not None and host.memory_available_bytes < MEMORY_WARNING_BYTES:
        warnings.append(f"内存只剩 {host.memory_available_bytes / MIB:.0f} MiB")
    if host.disk_free_bytes < DEPLOY_DISK_FLOOR_BYTES:
        warnings.append(f"磁盘只剩 {host.disk_free_bytes / GIB:.1f} GiB，少于 20 GiB 时发版部署的预检过不去")
    return warnings


def _identity_warnings(registrations: Registrations, emails: EmailCounts | None) -> list[str]:
    warnings: list[str] = []
    if registrations.today >= EMAIL_WARNING:
        warnings.append(
            f"今天已经注册了 {registrations.today} 个账户，每个发一封验证信，"
            f"快到 Resend 免费档每天 {EMAIL_DAILY_QUOTA} 封了，见预案“发信”"
        )
    if emails is not None and emails.failed:
        warnings.append(f"有 {emails.failed} 封信没发出去，看身份服务的日志")
    return warnings


def _single_json(output: str) -> dict[str, object]:
    lines = [line for line in output.splitlines() if line.strip()]
    if len(lines) != 1:
        raise LaunchStatusError("查询应当只回一行 JSON。")
    try:
        value = json.loads(lines[0])
    except json.JSONDecodeError as error:
        raise LaunchStatusError("查询回来的不是 JSON。") from error
    if not isinstance(value, dict):
        raise LaunchStatusError("查询回来的不是一个对象。")
    return value
