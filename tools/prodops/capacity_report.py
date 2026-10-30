"""每周一份容量报告（`production.py capacity-report`，设计在 `specs/measurement/design.md` 第 2 节）。

问本机的 Prometheus，拼成给人看的几张表；`--post` 时开成 `agent-room-ops` 的 issue，标 `capacity-report`，
不标 `prod-error`。整机的网络流量 Prometheus 里没有（主机指标在容器自己的网络里），由这里读主机的
`/proc/net/dev`，和上一次发报告时记下的比。报告里只有按地区、按库、按告警汇总的数，没有账户、房间和 IP。
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
import json
import os
from pathlib import Path
from typing import Protocol
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

WEEK = timedelta(days=7)
# 部署预检要的可用空间（`runtime.MINIMUM_DISK_BYTES`），盘还能撑几周按它算。
DISK_FLOOR_BYTES = 20 * 1024**3
STATE_VERSION = 1
LABEL = "capacity-report"
REQUEST_TIMEOUT_SECONDS = 30

# 进下一阶段的起点（设计的“什么时候进下一阶段”），报告最后照着说一句。
SYNAPSE_CORE_LIMIT = 0.7
AVAILABLE_MEMORY_FLOOR_BYTES = 500 * 1024**2
DISK_RUNWAY_FLOOR = timedelta(days=60)

REGIONS = {
    "cn": "中国大陆",
    "hk_mo_tw": "港澳台",
    "jp_kr": "日韩",
    "southeast_asia": "东南亚",
    "south_asia": "南亚",
    "oceania": "大洋洲",
    "europe": "欧洲",
    "north_america": "北美",
    "latin_america": "拉美",
    "middle_east_africa": "中东和非洲",
    "unknown": "未知",
}
KINDS = {"person": "人", "local_agent": "本机 Agent", "network_agent": "网络 Agent"}
DATABASES = {"agent_room": "控制面的库", "synapse": "Synapse 的库", "keycloak": "身份服务的库"}
# 本机容器之间的虚拟网卡不算进主机流量。
_VIRTUAL_INTERFACES = ("lo", "docker", "br-", "veth")

_HOST = 'job="host"'
_ROOT = f'{_HOST},mountpoint="/"'
QUERIES = {
    "accounts": "max by (region, kind) (max_over_time(agent_room_active_accounts[7d]))",
    "requests": "sum by (region, kind) (increase(agent_room_requests_by_region_total[7d]))",
    "samples": (
        'sum by (region) (increase(agent_room_frontend_duration_seconds_count{metric="api_round_trip"}[7d]))'
    ),
    "synapse_cpu": (
        'quantile_over_time(0.95, sum(rate(process_cpu_seconds_total{job="synapse"}[5m]))[7d:5m])'
    ),
    "host_cpu": (
        f'quantile_over_time(0.95, (1 - avg(rate(node_cpu_seconds_total{{{_HOST},mode="idle"}}[5m])))[7d:5m])'
    ),
    "cores": f'count(node_cpu_seconds_total{{{_HOST},mode="idle"}})',
    "memory_used": (
        f"quantile_over_time(0.95, (node_memory_MemTotal_bytes{{{_HOST}}}"
        f" - node_memory_MemAvailable_bytes{{{_HOST}}})[7d:5m])"
    ),
    "memory_available_min": f"min_over_time(node_memory_MemAvailable_bytes{{{_HOST}}}[7d])",
    "memory_total": f"node_memory_MemTotal_bytes{{{_HOST}}}",
    "events_per_minute": (
        "quantile_over_time(0.95, sum(rate(synapse_storage_events_persisted_events_sep_total[5m]))[7d:5m]) * 60"
    ),
    "lobby_slots": 'max_over_time(agent_room_operational_value{metric="public_lobby_occupied_slots"}[7d])',
    "lobby_agents": 'max_over_time(agent_room_operational_value{metric="public_lobby_network_agents"}[7d])',
    "emails": "max_over_time(agent_room:emails_sent:increase_24h[7d])",
    "alerts": 'sum by (alertname, severity) (count_over_time(ALERTS{alertstate="firing"}[7d]))',
}


def latency_query(metric: str, quantile: float) -> str:
    return (
        f"histogram_quantile({quantile}, sum by (le, region) "
        f'(increase(agent_room_frontend_duration_seconds_bucket{{metric="{metric}"}}[7d])))'
    )


# 存储一周前、现在各问一次，比一比涨了多少。
STORAGE = {
    "databases": "pg_database_size_bytes",
    "objects": "agent_room_backup_object_bytes",
    "backups": "agent_room_backup_repository_bytes",
    "prometheus": "sum(prometheus_tsdb_storage_blocks_bytes) + sum(prometheus_tsdb_wal_storage_size_bytes)",
    "disk_used": f"node_filesystem_size_bytes{{{_ROOT}}} - node_filesystem_avail_bytes{{{_ROOT}}}",
    "disk_free": f"node_filesystem_avail_bytes{{{_ROOT}}}",
}
# 最忙的一小时：按小时取一周，挑最大的那一格，连时间一起报。
BUSIEST = {
    "Synapse 存下的事件": "sum(increase(synapse_storage_events_persisted_events_sep_total[1h]))",
    "控制面的接口请求": "sum(increase(agent_room_api_server_requests_total[1h]))",
}


class CapacityReportError(RuntimeError):
    """状态文件坏了、Prometheus 读不到、发 issue 没成。"""


@dataclass(frozen=True, slots=True)
class Sample:
    labels: dict[str, str]
    value: float


class Prometheus(Protocol):
    def query(self, expression: str, at: datetime) -> list[Sample]: ...

    def query_range(
        self, expression: str, start: datetime, end: datetime, step: timedelta
    ) -> list[tuple[datetime, float]]: ...


class ReportTracker(Protocol):
    def create(self, title: str, body: str, labels: list[str]) -> int: ...

    def close(self, number: int, comment: str) -> None: ...


class HttpPrometheus:
    """本机端口上的 Prometheus HTTP 接口（`/api/v1/query`、`/api/v1/query_range`），只读。"""

    def __init__(self, base_url: str) -> None:
        self._base_url = base_url.rstrip("/")

    def query(self, expression: str, at: datetime) -> list[Sample]:
        data = self._get("/api/v1/query", {"query": expression, "time": f"{at.timestamp():.3f}"})
        return [
            Sample({str(key): str(value) for key, value in item.get("metric", {}).items()}, _number(item["value"][1]))
            for item in data.get("result", [])
            if isinstance(item, dict)
        ]

    def query_range(
        self, expression: str, start: datetime, end: datetime, step: timedelta
    ) -> list[tuple[datetime, float]]:
        parameters = {
            "query": expression,
            "start": f"{start.timestamp():.3f}",
            "end": f"{end.timestamp():.3f}",
            "step": f"{int(step.total_seconds())}s",
        }
        result = self._get("/api/v1/query_range", parameters).get("result", [])
        if not result:
            return []
        return [(datetime.fromtimestamp(float(moment), UTC), _number(value)) for moment, value in result[0]["values"]]

    def _get(self, path: str, parameters: dict[str, str]) -> dict[str, object]:
        request = Request(f"{self._base_url}{path}?{urlencode(parameters)}", headers={"Accept": "application/json"})
        try:
            with urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
                document = json.load(response)
        except HTTPError as error:
            raise CapacityReportError(f"回 {error.code}") from None
        except (URLError, TimeoutError, OSError, ValueError) as error:
            raise CapacityReportError(error.__class__.__name__) from None
        data = document.get("data") if isinstance(document, dict) else None
        if not isinstance(data, dict):
            raise CapacityReportError("回答不对")
        return data


def _number(value: object) -> float:
    try:
        return float(str(value))
    except ValueError:
        return float("nan")


@dataclass(frozen=True, slots=True)
class NetworkReading:
    """主机网卡从开机到现在一共收发了多少字节。换了开机号（重启过）就没法和上一次比。"""

    at: datetime
    boot_id: str
    received: int
    sent: int

    def to_json(self) -> dict[str, object]:
        return {"at": _iso(self.at), "bootId": self.boot_id, "received": self.received, "sent": self.sent}

    @classmethod
    def from_json(cls, value: object) -> NetworkReading | None:
        if not isinstance(value, dict):
            return None
        try:
            return cls(
                datetime.fromisoformat(str(value["at"]).replace("Z", "+00:00")),
                str(value["bootId"]),
                int(value["received"]),
                int(value["sent"]),
            )
        except (KeyError, TypeError, ValueError):
            return None


def read_network(now: datetime, proc: Path = Path("/proc")) -> NetworkReading | None:
    """读主机的 `/proc/net/dev`，不算回环和本机容器的虚拟网卡。不是 Linux 就没有。"""

    try:
        table = (proc / "net" / "dev").read_text(encoding="utf-8")
        boot_id = (proc / "sys" / "kernel" / "random" / "boot_id").read_text(encoding="utf-8").strip()
    except OSError:
        return None
    received = sent = 0
    for line in table.splitlines()[2:]:
        name, _, counters = line.partition(":")
        fields = counters.split()
        if name.strip().startswith(_VIRTUAL_INTERFACES) or len(fields) < 9:
            continue
        received += int(fields[0])
        sent += int(fields[8])
    return NetworkReading(now, boot_id, received, sent)


@dataclass(slots=True)
class ReportState:
    """上一次发报告时记下的：网卡读数、哪一周发过、发成了几号 issue。只在 `--post` 时写。"""

    network: NetworkReading | None = None
    last_week: str | None = None
    last_issue: int | None = None

    @classmethod
    def load(cls, path: Path) -> ReportState:
        if not path.exists():
            return cls()
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except ValueError as error:
            raise CapacityReportError(f"认不出容量报告的状态文件：{path}") from error
        if not isinstance(data, dict) or data.get("version") != STATE_VERSION:
            raise CapacityReportError(f"认不出容量报告的状态文件：{path}")
        issue = data.get("lastIssue")
        week = data.get("lastWeek")
        return cls(
            network=NetworkReading.from_json(data.get("network")),
            last_week=str(week) if week else None,
            last_issue=int(issue) if isinstance(issue, int) else None,
        )

    def save(self, path: Path) -> None:
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        document = {
            "version": STATE_VERSION,
            "network": None if self.network is None else self.network.to_json(),
            "lastWeek": self.last_week,
            "lastIssue": self.last_issue,
        }
        temporary.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)


def week_label(now: datetime) -> str:
    """报告管的是到 `now` 为止的七天，按这七天里头一天所在的 ISO 周记。"""

    year, week, _ = (now - WEEK).isocalendar()
    return f"{year}-W{week:02d}"


def title(now: datetime) -> str:
    return f"容量周报 {(now - WEEK):%Y-%m-%d} – {now:%Y-%m-%d}"


def build_report(
    prometheus: Prometheus,
    *,
    now: datetime,
    network: NetworkReading | None,
    previous_network: NetworkReading | None,
    version: str,
) -> list[str]:
    """给人看的报告，每一行都只有汇总的数。Prometheus 读不到时照样出网络那一节，并写明读不到。"""

    lines = [
        f"# {title(now)}",
        "",
        f"版本 {version}；数从本机的 Prometheus 来（留 45 天），这份报告就是长期的记录。"
        "设计在 `specs/measurement/design.md`。",
        "",
    ]
    try:
        values = {name: prometheus.query(expression, now) for name, expression in QUERIES.items()}
        latency = {
            (metric, quantile): prometheus.query(latency_query(metric, quantile), now)
            for metric in ("api_round_trip", "largest_contentful_paint")
            for quantile in (0.5, 0.95)
        }
        storage = {
            name: (prometheus.query(expression, now), prometheus.query(expression, now - WEEK))
            for name, expression in STORAGE.items()
        }
        busiest = {
            name: prometheus.query_range(expression, now - WEEK, now, timedelta(hours=1))
            for name, expression in BUSIEST.items()
        }
    except CapacityReportError as error:
        lines.extend([f"**Prometheus 读不到（{error}），这周只有网络流量。**", ""])
        lines.extend(_network_section(network, previous_network))
        return lines
    lines.extend(_usage_section(values, latency))
    lines.extend(_machine_section(values))
    storage_lines, disk_runway = _storage_section(storage)
    lines.extend(storage_lines)
    lines.extend(_activity_section(values, busiest))
    lines.extend(_network_section(network, previous_network))
    lines.extend(_alert_section(values["alerts"]))
    lines.extend(_verdict(values, disk_runway))
    lines.extend(
        [
            "## 数不准的地方",
            "",
            "- 用代理、VPN 的人算在出口那个地区，中国大陆会被算少。",
            "- 没登录的访客（官网、围观页）不算进地区里。",
            "- 每天的活跃账户记在控制面的内存里，控制面重启（比如发版）那天会偏小。",
        ]
    )
    return lines


def _usage_section(values: dict[str, list[Sample]], latency: dict[tuple[str, float], list[Sample]]) -> list[str]:
    accounts = _by(values["accounts"], "region", "kind")
    requests = _by(values["requests"], "region", "kind")
    # 控制面每个地区、每种账户都报，没人来的是 0：只列这周真有人来过的地区。
    regions = sorted(
        {region for (region, _), value in [*accounts.items(), *requests.items()] if value > 0},
        key=lambda region: -sum(value for (other, _), value in accounts.items() if other == region),
    )
    lines = ["## 谁在用、快不快", ""]
    if not regions:
        if accounts or requests:
            return [*lines, "这周哪个地区都没有登录的人和 Agent 来过。", ""]
        return [*lines, "还没有按地区的数（地区汇总上线、有人用过以后才有）。", ""]
    lines.extend(
        [
            "账户是这周最多的那一天有几个在用；请求是这一周加起来。",
            "",
            "| 地区 | 人 | 本机 Agent | 网络 Agent | 请求（人 / 本机 Agent / 网络 Agent） |",
            "| --- | ---: | ---: | ---: | --- |",
        ]
    )
    for region in regions:
        counts = " | ".join(_count(accounts.get((region, kind))) for kind in KINDS)
        calls = " / ".join(_count(requests.get((region, kind))) for kind in KINDS)
        lines.append(f"| {_region(region)} | {counts} | {calls} |")
    samples = _by(values["samples"], "region")
    quantiles = {key: _by(value, "region") for key, value in latency.items()}
    lines.extend(
        [
            "",
            "快慢是这一周的 P50 / P95，毫秒。“接口来回”是网页打开时调一次很轻的接口，看网络本身；"
            "“打开页面”是最大一块内容画出来的时间。",
            "",
            "| 地区 | 接口来回 | 打开页面 | 样本 |",
            "| --- | --- | --- | ---: |",
        ]
    )
    for region in regions:
        cells = [
            f"{_milliseconds(quantiles[(metric, 0.5)].get((region,)))} / "
            f"{_milliseconds(quantiles[(metric, 0.95)].get((region,)))}"
            for metric in ("api_round_trip", "largest_contentful_paint")
        ]
        lines.append(f"| {_region(region)} | {cells[0]} | {cells[1]} | {_count(samples.get((region,)))} |")
    return [*lines, ""]


def _machine_section(values: dict[str, list[Sample]]) -> list[str]:
    cores = _single(values["cores"])
    host_cpu = _single(values["host_cpu"])
    synapse = _single(values["synapse_cpu"])
    total = _single(values["memory_total"])
    used = _single(values["memory_used"])
    available = _single(values["memory_available_min"])
    cpu = "还没有数" if host_cpu is None else f"{host_cpu:.0%}" + (f"（{cores:.0f} 个核）" if cores else "")
    return [
        "## 机器",
        "",
        f"- Synapse 的 CPU，P95：{'还没有数' if synapse is None else f'{synapse:.2f} 个核'}",
        f"- 整机 CPU，P95：{cpu}",
        f"- 内存用了，P95：{_size(used)}" + (f"，共 {_size(total)}" if total else ""),
        f"- 可用内存最低：{_size(available)}",
        "",
    ]


def _storage_section(
    storage: dict[str, tuple[list[Sample], list[Sample]]],
) -> tuple[list[str], timedelta | None]:
    lines = ["## 存储", "", "| | 现在 | 一周前 | 涨了 |", "| --- | ---: | ---: | ---: |"]
    now_databases, before_databases = (_by(samples, "datname") for samples in storage["databases"])
    rows = [
        (label, now_databases.get((name,)), before_databases.get((name,))) for name, label in DATABASES.items()
    ]
    for name, label in (
        ("objects", "对象（附件和正文，按备份里的镜像）"),
        ("backups", "备份仓库"),
        ("prometheus", "Prometheus"),
        ("disk_used", "盘已用"),
    ):
        current, before = storage[name]
        rows.append((label, _single(current), _single(before)))
    for label, current, before in rows:
        growth = None if current is None or before is None else current - before
        lines.append(f"| {label} | {_size(current)} | {_size(before)} | {_size(growth, signed=True)} |")
    free = _single(storage["disk_free"][0])
    current_used, before_used = (_single(samples) for samples in storage["disk_used"])
    weekly = None if current_used is None or before_used is None else current_used - before_used
    lines.append("")
    if free is None:
        return [*lines, "盘还剩多少：还没有数。", ""], None
    if weekly is None or weekly <= 0:
        return [*lines, f"盘还剩 {_size(free)}；这周没涨（或还没有一周前的数）。", ""], None
    # 涨得很慢时周数会大得没边，封顶十年。
    runway = WEEK * min(max(free - DISK_FLOOR_BYTES, 0) / weekly, 520)
    return [
        *lines,
        f"盘还剩 {_size(free)}；照这周的速度，约 {runway.days // 7} 周后低于部署预检要的 20 GiB。",
        "",
    ], runway


def _activity_section(
    values: dict[str, list[Sample]], busiest: dict[str, list[tuple[datetime, float]]]
) -> list[str]:
    events = _single(values["events_per_minute"])
    lines = [
        "## 忙不忙",
        "",
        f"- Synapse 每分钟存下的事件（所有房间的消息、进出房间都算），P95：{_count(events)}",
    ]
    for name, points in busiest.items():
        points = [(moment, value) for moment, value in points if value == value]
        if not points:
            lines.append(f"- {name}最忙的一小时：还没有数")
            continue
        moment, value = max(points, key=lambda point: point[1])
        lines.append(f"- {name}最忙的一小时：{_count(value)} 个，到 {moment:%m-%d %H:%M} UTC 为止")
    lines.extend(
        [
            f"- 公开大厅占着的位置，最多：{_count(_single(values['lobby_slots']))}；"
            f"其中网络 Agent 最多：{_count(_single(values['lobby_agents']))}",
            f"- 24 小时里发的验证信和重设密码信，最多：{_count(_single(values['emails']))} 封",
            "",
        ]
    )
    return lines


def _network_section(current: NetworkReading | None, previous: NetworkReading | None) -> list[str]:
    lines = ["## 主机网络流量", ""]
    if current is None:
        return [*lines, "这台机器读不到 `/proc/net/dev`。", ""]
    if previous is None:
        return [*lines, "第一次记，下一份报告才有一周的数。", ""]
    if previous.boot_id != current.boot_id or current.received < previous.received or current.sent < previous.sent:
        return [*lines, "上次记下以后服务器重启过，这一周算不出来；下一份照常。", ""]
    span = current.at - previous.at
    days = span.total_seconds() / 86400
    return [
        *lines,
        f"{previous.at:%m-%d %H:%M} 到 {current.at:%m-%d %H:%M} UTC（{days:.1f} 天）："
        f"进 {_size(current.received - previous.received)}，出 {_size(current.sent - previous.sent)}。"
        "只算主机的网卡，不算本机容器之间的。",
        "",
    ]


def _alert_section(alerts: list[Sample]) -> list[str]:
    lines = ["## 这周响过的告警", ""]
    if not alerts:
        return [*lines, "没有。", ""]
    lines.extend(["| 告警 | 轻重 | 响了多久 |", "| --- | --- | ---: |"])
    for sample in sorted(alerts, key=lambda sample: -sample.value):
        name = sample.labels.get("alertname", "?")
        # 规则每分钟算一次，ALERTS 每分钟一个点。
        lines.append(f"| {name} | {sample.labels.get('severity', '?')} | 约 {sample.value:.0f} 分钟 |")
    return [*lines, ""]


def _verdict(values: dict[str, list[Sample]], disk_runway: timedelta | None) -> list[str]:
    warnings = []
    synapse = _single(values["synapse_cpu"])
    if synapse is not None and synapse > SYNAPSE_CORE_LIMIT:
        warnings.append(f"Synapse 的 CPU P95 有 {synapse:.2f} 个核，超过了 {SYNAPSE_CORE_LIMIT} 个核。")
    available = _single(values["memory_available_min"])
    if available is not None and available < AVAILABLE_MEMORY_FLOOR_BYTES:
        warnings.append(f"可用内存最低到了 {_size(available)}，低于 500 MB。")
    if disk_runway is not None and disk_runway < DISK_RUNWAY_FLOOR:
        warnings.append("照这周的速度，盘 60 天内就不够部署预检要的 20 GiB 了。")
    lines = ["## 要不要进下一阶段", ""]
    if not warnings:
        return [*lines, "都还在设计里定的起点以下。", ""]
    return [
        *lines,
        *(f"- {warning}" for warning in warnings),
        "",
        "按设计，连着出现就该开始阶段 1（拆 Synapse workers、换大一点的机器、对象存储搬到托管 S3）。",
        "",
    ]


def post_report(tracker: ReportTracker, state: ReportState, lines: list[str], *, now: datetime) -> int | None:
    """开这一周的 issue，关掉上一周的。这一周已经发过（比如重跑）就不再发，回 None。"""

    week = week_label(now)
    if state.last_week == week:
        return None
    body = "\n".join(
        [
            f"<!-- agent-room-capacity-report: {week} -->",
            *lines,
            "",
            "（`production.py capacity-report --post` 每周一自动开的，只有汇总的数。新的一份开出来时这一份自动关掉。）",
        ]
    )
    number = tracker.create(title(now), body, [LABEL])
    previous = state.last_issue
    state.last_week = week
    state.last_issue = number
    if previous is not None:
        tracker.close(previous, f"下一份是 #{number}。")
    return number


def _by(samples: list[Sample], *labels: str) -> dict[tuple[str, ...], float]:
    return {tuple(sample.labels.get(label, "") for label in labels): sample.value for sample in samples}


def _single(samples: list[Sample]) -> float | None:
    values = [sample.value for sample in samples if sample.value == sample.value]
    return sum(values) if values else None


def _region(region: str) -> str:
    return REGIONS.get(region, "未知")


def _count(value: float | None) -> str:
    return "–" if value is None or value != value else f"{value:,.0f}"


def _milliseconds(seconds: float | None) -> str:
    if seconds is None or seconds != seconds or seconds == float("inf"):
        return "–"
    return f"{seconds * 1000:,.0f}"


def _size(value: float | None, *, signed: bool = False) -> str:
    if value is None or value != value:
        return "–"
    sign = ("+" if value >= 0 else "−") if signed else ""
    magnitude = abs(value)
    for unit, scale in (("GiB", 1024**3), ("MiB", 1024**2), ("KiB", 1024)):
        if magnitude >= scale:
            return f"{sign}{magnitude / scale:.1f} {unit}"
    return f"{sign}{magnitude:.0f} B"


def _iso(moment: datetime) -> str:
    return moment.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
