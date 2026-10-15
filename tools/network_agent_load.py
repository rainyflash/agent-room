"""量一量网络 Agent 多了以后服务器撑得住多少（specs/launch-capacity/design.md 第 6 步）。

和无头验收用同一套隔离的依赖（PostgreSQL、Synapse、Keycloak），起一个发布构建的控制面，用部署配置同款的
环境变量放开公开大厅的限额、连接池和依赖超时和生产一样。每一档建够 N 个网络 Agent，让它们一直等消息；再从
它们里面随机挑人，每秒在大厅里说 M 句，三成点名别人、三成说之前先“正在输入”。量：

- Synapse、PostgreSQL、控制面各用了几个核、内存最多用到多少；
- 消息多久送到（中位数、p95）；
- 出错的个数（按状态码和超时分），建 Agent 时重试了几次、每个建了多久。

等消息分两种叫醒方式各量一遍：
- `related`：默认的规则，跟它有关的才叫醒。大厅里只有点名它的那句会叫醒它，量的是点名到被点名的收到；
- `all`：每句话都叫醒每个在等的，最重的情形，量的是每个在等的都收到。

只在 CI 里派发跑（`gh workflow run ci.yml -f suite=load`），结果写到
artifacts/capacity/network-agent-load.json。CI 的机器比生产快，所以看用了几个核：Synapse 超过约 0.7 个
核、三样加起来超过约 1.5 个核，就算生产这台 2 vCPU 的撑不住。
"""

from __future__ import annotations

import argparse
from collections import Counter
from collections.abc import Callable, Iterable, Mapping, Sequence
from concurrent.futures import Future, ThreadPoolExecutor
from dataclasses import dataclass, field
from http.client import HTTPException
import json
import math
import os
from pathlib import Path
import random
import re
import secrets
import subprocess
import sys
import threading
import time
from typing import Final
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import Request, urlopen

if __package__:
    from . import vertical as v
else:
    import vertical as v

AGENT_COUNTS: Final = (50, 100, 200, 400)
MESSAGE_RATES: Final = (0.2, 1.0)
WAKE_MODES: Final = ("related", "all")
WAIT_QUERIES: Final = {
    "related": "wait=10&limit=50&settle=0",
    "all": "wait=10&limit=50&wake=all&settle=0",
}
WINDOW_SECONDS: Final = 60.0
IDLE_WINDOW_SECONDS: Final = 30.0
GRACE_SECONDS: Final = 15.0
SETTLE_SECONDS: Final = 15.0
MENTION_SHARE: Final = 0.3
TYPING_SHARE: Final = 0.3
SYNAPSE_CORE_BUDGET: Final = 0.7
TOTAL_CORE_BUDGET: Final = 1.5
CREATE_WORKERS: Final = 4
CREATE_ATTEMPTS: Final = 6
SEND_WORKERS: Final = 8
MATRIX_API: Final = "http://127.0.0.1:18008/_matrix/client/v3"
STATUS_EVENT_TYPE: Final = "io.github.rainyflash.agentroom.agent.status.v1"
REPORT: Final = v.ROOT / "artifacts" / "capacity" / "network-agent-load.json"
CONTROL_PLANE_LOG: Final = v.LOG_ROOT / "control-plane-load.log"
PRODUCTION_LIKE: Final = {
    # 生产渲染给一个控制面副本的连接池（tools/prodops/render.py 的 control_plane_database_connections）
    # 和依赖超时（infra/production/compose.yaml）；本地默认的 2 秒会把排队的进大厅请求提前判成 503。
    "AGENT_ROOM_DB_MAX_CONNECTIONS": "20",
    "AGENT_ROOM_DEPENDENCY_TIMEOUT_MS": "5000",
}
# 部署配置 networkAgents.limits 渲染成的环境变量（tools/prodops/config.py 的 NETWORK_AGENT_LIMIT_FIELDS）
# 放到最大：量的是服务器，不是限额。
LIMIT_OVERRIDES: Final = {
    "AGENT_ROOM_NETWORK_AGENT_LOBBY_ENTRIES_PER_HOUR": "100000",
    "AGENT_ROOM_NETWORK_AGENT_LOBBY_ENTRIES_PER_DAY": "1000000",
    "AGENT_ROOM_NETWORK_AGENT_MAX_LOBBY_AGENTS": "100000",
    "AGENT_ROOM_NETWORK_AGENT_LOBBY_MESSAGES_PER_MINUTE": "600",
    "AGENT_ROOM_NETWORK_AGENT_LOBBY_MESSAGES_PER_DAY": "1000000",
}
# 生产的控制面同款日志级别（infra/production/compose.yaml）。
CONTROL_PLANE_LOG_FILTER: Final = "agent_room_control_plane=info,sqlx=warn"
LOAD_TEXT: Final = re.compile(r"^load (?P<nonce>[0-9a-f]{16}) (?P<speaker>\d+)$")
LOAD_NAME: Final = re.compile(r"\bLoad (?P<index>\d{3,})\b")


@dataclass
class LoadAgent:
    index: int
    token: str
    room_id: str

    @property
    def source(self) -> str:
        # 198.18.0.0/15 是留给测量用的地址段；每个 Agent 一个来源，和真实首发时差不多。
        return f"198.18.{self.index // 200}.{self.index % 200 + 1}"


@dataclass(frozen=True)
class Sent:
    at: float
    room_id: str
    speaker: int
    target: int | None


@dataclass
class Ledger:
    """一段里发了什么、谁什么时候收到、出了什么错。等消息的线程和发言的线程一起写，所以加锁。"""

    _lock: threading.Lock = field(default_factory=threading.Lock)
    _wake: str = "all"
    _sent: dict[str, Sent] = field(default_factory=dict)
    _delivered: set[tuple[str, int]] = field(default_factory=set)
    _latencies: list[float] = field(default_factory=list)
    _errors: Counter[str] = field(default_factory=Counter)
    # 说过话的 Agent 的 Matrix 用户 ID：点名、“正在输入”都要它，从收到的消息里学来，跨段留着。
    _user_ids: dict[int, str] = field(default_factory=dict)

    def begin(self, wake: str) -> None:
        with self._lock:
            self._wake = wake
            self._sent.clear()
            self._delivered.clear()
            self._latencies.clear()
            self._errors.clear()

    def sending(self, nonce: str, room_id: str, speaker: int, target: int | None) -> None:
        with self._lock:
            self._sent[nonce] = Sent(time.monotonic(), room_id, speaker, target)

    def not_sent(self, nonce: str, reason: str) -> None:
        with self._lock:
            self._sent.pop(nonce, None)
            self._errors[reason] += 1

    def error(self, reason: str) -> None:
        with self._lock:
            self._errors[reason] += 1

    def received(self, nonce: str, receiver: int, speaker: int, author: str | None) -> None:
        now = time.monotonic()
        with self._lock:
            if author is not None:
                self._user_ids.setdefault(speaker, author)
            sent = self._sent.get(nonce)
            if sent is None or not self._counts(sent, receiver) or (nonce, receiver) in self._delivered:
                return
            self._delivered.add((nonce, receiver))
            self._latencies.append(now - sent.at)

    def _counts(self, sent: Sent, receiver: int) -> bool:
        if self._wake == "related":
            return receiver == sent.target
        return receiver != sent.speaker

    def learn(self, index: int, user_id: str) -> None:
        with self._lock:
            self._user_ids.setdefault(index, user_id)

    def user_id(self, index: int) -> str | None:
        with self._lock:
            return self._user_ids.get(index)

    def summary(self, room_sizes: Mapping[str, int]) -> dict[str, object]:
        with self._lock:
            if self._wake == "related":
                expected = sum(1 for sent in self._sent.values() if sent.target is not None)
            else:
                expected = sum(max(room_sizes.get(sent.room_id, 0) - 1, 0) for sent in self._sent.values())
            return {
                "messagesSent": len(self._sent),
                "deliveriesExpected": expected,
                "deliveriesReceived": len(self._latencies),
                "latencySeconds": latency_summary(self._latencies),
                "errors": dict(sorted(self._errors.items())),
            }


def latency_summary(latencies: Sequence[float]) -> dict[str, float] | None:
    if not latencies:
        return None
    ordered = sorted(latencies)
    return {
        "p50": round(percentile(ordered, 0.5), 3),
        "p95": round(percentile(ordered, 0.95), 3),
        "max": round(ordered[-1], 3),
    }


def percentile(ordered: Sequence[float], share: float) -> float:
    """最近秩法：至少有 share 那么多的值不大于它。"""
    rank = max(math.ceil(share * len(ordered)), 1)
    return ordered[rank - 1]


def parse_load_text(text: object) -> tuple[str, int] | None:
    if not isinstance(text, str):
        return None
    match = LOAD_TEXT.match(text)
    if match is None:
        return None
    return match.group("nonce"), int(match.group("speaker"))


def within_budget(cores: Mapping[str, float]) -> bool:
    return cores["synapse"] <= SYNAPSE_CORE_BUDGET and cores["total"] <= TOTAL_CORE_BUDGET


def create_backoff(attempt: int) -> float:
    return float(min(2**attempt, 16))


# ---- 容器和进程用了多少 CPU、内存 ----


def cgroup_files(proc_cgroup: str) -> tuple[Path, Path]:
    """从 /proc/<pid>/cgroup 找到记 CPU 和内存的文件：cgroup v2 是 cpu.stat 和 memory.current，
    v1 是 cpuacct.usage 和 memory.usage_in_bytes。"""
    root = Path("/sys/fs/cgroup")
    v1_cpu: Path | None = None
    v1_memory: Path | None = None
    for line in proc_cgroup.splitlines():
        hierarchy, controllers, path = line.split(":", 2)
        relative = path.lstrip("/")
        if hierarchy == "0" and controllers == "":
            return root / relative / "cpu.stat", root / relative / "memory.current"
        names = controllers.split(",")
        if "cpuacct" in names:
            v1_cpu = root / controllers / relative / "cpuacct.usage"
        if "memory" in names:
            v1_memory = root / "memory" / relative / "memory.usage_in_bytes"
    if v1_cpu is None or v1_memory is None:
        raise v.VerticalFailure("找不到容器的 cgroup，量不了 CPU。")
    return v1_cpu, v1_memory


def cgroup_cpu_seconds(path: Path) -> float:
    content = path.read_text(encoding="utf-8")
    if path.name == "cpu.stat":
        for line in content.splitlines():
            key, value = line.split()
            if key == "usage_usec":
                return int(value) / 1_000_000
        raise v.VerticalFailure(f"{path} 里没有 usage_usec。")
    return int(content.strip()) / 1_000_000_000


def process_cpu_seconds(pid: int) -> float:
    stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    fields = stat.rsplit(")", 1)[1].split()
    # 括号后面从第 3 个字段（状态）算起：utime 是第 14 个，stime 是第 15 个。
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def process_memory_bytes(pid: int) -> int:
    for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1]) * 1024
    raise v.VerticalFailure("读不到控制面的内存。")


@dataclass(frozen=True)
class Meter:
    containers: Mapping[str, tuple[Path, Path]]
    control_plane_pid: int

    @classmethod
    def attach(cls, control_plane_pid: int) -> Meter:
        containers = {}
        for name, service in (("synapse", "synapse"), ("postgres", "postgres")):
            pid = container_pid(service)
            cgroup = Path(f"/proc/{pid}/cgroup").read_text(encoding="utf-8")
            containers[name] = cgroup_files(cgroup)
        return cls(containers=containers, control_plane_pid=control_plane_pid)

    def cpu_seconds(self) -> dict[str, float]:
        usage = {name: cgroup_cpu_seconds(files[0]) for name, files in self.containers.items()}
        usage["controlPlane"] = process_cpu_seconds(self.control_plane_pid)
        # 量的工具自己（几百个等消息的线程）也占 CI 机器的核；单独记，不算进合计。
        usage["harness"] = process_cpu_seconds(os.getpid())
        return usage

    def memory_bytes(self) -> dict[str, int]:
        usage = {
            name: int(files[1].read_text(encoding="utf-8").strip())
            for name, files in self.containers.items()
        }
        usage["controlPlane"] = process_memory_bytes(self.control_plane_pid)
        return usage


def container_pid(service: str) -> int:
    container = subprocess.run(
        [*v.compose_command(v.VERTICAL_PROJECT_NAME), "ps", "-q", service],
        cwd=v.ROOT, capture_output=True, text=True, check=True,
    ).stdout.strip()
    if not container:
        raise v.VerticalFailure(f"隔离环境里没有 {service} 容器。")
    pid = subprocess.run(
        [v.executable("docker"), "inspect", "--format", "{{.State.Pid}}", container],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    return int(pid)


class PeakMemory:
    """每 2 秒看一次内存，记下每样最多用到多少。"""

    def __init__(self, meter: Meter) -> None:
        self._meter = meter
        self._stop = threading.Event()
        self._peak: dict[str, int] = {}
        self._thread = threading.Thread(target=self._run, name="peak-memory", daemon=True)

    def __enter__(self) -> PeakMemory:
        self._thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self._stop.set()
        self._thread.join(timeout=10)

    def _run(self) -> None:
        while not self._stop.is_set():
            for name, value in self._meter.memory_bytes().items():
                self._peak[name] = max(self._peak.get(name, 0), value)
            self._stop.wait(2)

    def mebibytes(self) -> dict[str, int]:
        return {name: round(value / (1024 * 1024)) for name, value in self._peak.items()}


# ---- 网络 Agent：建、等消息、说话 ----


@dataclass
class Growth:
    """建 Agent 时的记账：每个建了多久、因为什么重试了几次。几个建的线程一起写。"""

    _lock: threading.Lock = field(default_factory=threading.Lock)
    _seconds: list[float] = field(default_factory=list)
    _retries: Counter[str] = field(default_factory=Counter)

    def took(self, seconds: float) -> None:
        with self._lock:
            self._seconds.append(seconds)

    def retried(self, reason: str) -> None:
        with self._lock:
            self._retries[reason] += 1

    def summary(self, agents: int) -> dict[str, object]:
        with self._lock:
            result = {
                "agents": agents,
                "createSeconds": latency_summary(self._seconds),
                "createRetries": dict(sorted(self._retries.items())),
            }
            self._seconds.clear()
            self._retries.clear()
            return result


def create_agent(index: int, growth: Growth) -> LoadAgent:
    """像真的网络 Agent 一样进大厅：503 时等一会再试，重试记下来，那就是首发时一拥而上会碰到的。"""
    agent = LoadAgent(index=index, token="", room_id="")
    for attempt in range(CREATE_ATTEMPTS):
        started = time.monotonic()
        try:
            status, created = v.network_agent_request(
                "POST", "", body={"name": f"Load {index:03d}", "room": v.CATALOG_SLUG},
                source=agent.source, timeout_seconds=120,
            )
        except (OSError, HTTPException) as error:
            status, created, reason = 0, None, type(error).__name__
        else:
            reason = f"HTTP {status} {created.get('code') if created else ''}".strip()
        if status == 201 and created is not None:
            growth.took(time.monotonic() - started)
            break
        if attempt + 1 == CREATE_ATTEMPTS or status not in (0, 429, 503):
            raise v.VerticalFailure(f"第 {index} 个网络 Agent 没建成：{reason}。")
        growth.retried(reason)
        time.sleep(create_backoff(attempt))
    agent.token = v.require_text(created.get("token"), "网络 Agent 令牌")
    room = v.require_object(created.get("room"), "进的房间")
    agent.room_id = v.require_text(room.get("matrixRoomId"), "房间 ID")
    # 进门时带回的上下文先确认掉，量的时候只看新说的。
    status, page = v.network_agent_request(
        "GET", "/me/messages?wait=0&limit=50", token=agent.token, source=agent.source
    )
    if status != 200 or page is None:
        raise v.VerticalFailure(f"第 {index} 个网络 Agent 取消息失败：HTTP {status}。")
    acknowledge(agent, v.network_page_messages(page))
    return agent


def acknowledge(agent: LoadAgent, messages: Sequence[Mapping[str, object]]) -> int:
    if not messages:
        return 200
    last = v.require_text(messages[-1].get("eventId"), "事件 ID")
    status, _ = v.network_agent_request(
        "POST", "/me/ack", token=agent.token, body={"eventId": last}, source=agent.source
    )
    return status


def wait_loop(agent: LoadAgent, run: LoadRun) -> None:
    """像一直在等消息的 Agent 一样：长轮询、收到就确认、接着等。叫醒方式每一轮读一次，量的时候能切换。"""
    ledger = run.ledger
    while not run.stop.is_set():
        try:
            status, page = v.network_agent_request(
                "GET", f"/me/messages?{WAIT_QUERIES[run.wake]}", token=agent.token,
                source=agent.source, timeout_seconds=40,
            )
        except (OSError, HTTPException) as error:  # 超时、连接被拒、断在半截都在这里
            ledger.error(f"wait {type(error).__name__}")
            run.stop.wait(1)
            continue
        if status != 200 or page is None:
            ledger.error(f"wait HTTP {status}")
            run.stop.wait(1)
            continue
        try:
            messages = v.network_page_messages(page)
        except v.VerticalFailure:
            ledger.error("wait malformed page")
            continue
        for message in messages:
            record_delivery(agent, ledger, message)
        try:
            status = acknowledge(agent, messages)
        except (OSError, HTTPException) as error:
            ledger.error(f"ack {type(error).__name__}")
            continue
        if status != 200:
            ledger.error(f"ack HTTP {status}")


def record_delivery(agent: LoadAgent, ledger: Ledger, message: Mapping[str, object]) -> None:
    conversation = message.get("conversation")
    parsed = parse_load_text(conversation.get("text") if isinstance(conversation, dict) else None)
    if parsed is None:
        return
    nonce, speaker = parsed
    try:
        author: str | None = v.message_author(message)
    except v.VerticalFailure:
        author = None
    ledger.received(nonce, agent.index, speaker, author)


def say(
    speaker: LoadAgent,
    ledger: Ledger,
    *,
    mention: tuple[int, str] | None,
    typing: bool,
    appservice_token: str,
) -> None:
    nonce = secrets.token_hex(8)
    user_id = ledger.user_id(speaker.index)
    if typing and user_id is not None:
        put_typing(appservice_token, speaker.room_id, user_id, ledger)
        time.sleep(1)
    ledger.sending(nonce, speaker.room_id, speaker.index, mention[0] if mention else None)
    try:
        status, _ = v.network_agent_request(
            "POST", "/me/messages", token=speaker.token,
            body={"text": f"load {nonce} {speaker.index}", "mentions": [mention[1]] if mention else []},
            source=speaker.source, timeout_seconds=60,
        )
    except (OSError, HTTPException) as error:
        ledger.not_sent(nonce, f"send {type(error).__name__}")
        return
    if status == 202:
        # Matrix 还没确认，多半随后就到：照样等它，单独记一笔。
        ledger.error("send accepted (202)")
    elif status != 201:
        ledger.not_sent(nonce, f"send HTTP {status}")


def put_typing(token: str, room_id: str, user_id: str, ledger: Ledger) -> None:
    """以应用服务的身份替这个 Agent 报“正在输入”：网络 Agent 自己没有这个接口，真实大厅里人会打字。"""
    path = (
        f"/rooms/{quote(room_id, safe='')}/typing/{quote(user_id, safe='')}"
        f"?user_id={quote(user_id, safe='')}"
    )
    request = Request(
        f"{MATRIX_API}{path}",
        data=json.dumps({"typing": True, "timeout": 3000}).encode("utf-8"),
        method="PUT",
    )
    request.add_header("Authorization", f"Bearer {token}")
    request.add_header("Content-Type", "application/json")
    try:
        with urlopen(request, timeout=30) as response:
            response.read()
    except HTTPError as error:
        ledger.error(f"typing HTTP {error.code}")
    except (OSError, HTTPException) as error:
        ledger.error(f"typing {type(error).__name__}")


def speak(
    agents: Sequence[LoadAgent],
    ledger: Ledger,
    *,
    rate: float,
    window: float,
    appservice_token: str,
    rng: random.Random,
) -> None:
    """按每秒 rate 句的节奏，从在等的 Agent 里随机挑人说话；发言放进线程池，慢了也不拖节奏。"""
    rooms: dict[str, list[LoadAgent]] = {}
    for agent in agents:
        rooms.setdefault(agent.room_id, []).append(agent)
    started = time.monotonic()
    pending: list[Future[None]] = []
    with ThreadPoolExecutor(SEND_WORKERS, thread_name_prefix="speaker") as pool:
        sent = 0
        while time.monotonic() - started < window:
            speaker = rng.choice(agents)
            pending.append(pool.submit(
                say, speaker, ledger,
                mention=pick_mention(speaker, rooms[speaker.room_id], ledger, rng),
                typing=rng.random() < TYPING_SHARE,
                appservice_token=appservice_token,
            ))
            sent += 1
            time.sleep(max(started + sent / rate - time.monotonic(), 0))
        for future in pending:
            future.result()


def pick_mention(
    speaker: LoadAgent, room: Sequence[LoadAgent], ledger: Ledger, rng: random.Random
) -> tuple[int, str] | None:
    """三成的话点名同一个分片里的另一个 Agent（得先知道它的 Matrix 用户 ID）。"""
    if rng.random() >= MENTION_SHARE:
        return None
    known = [
        (other.index, user_id)
        for other in room
        if other.index != speaker.index and (user_id := ledger.user_id(other.index)) is not None
    ]
    return rng.choice(known) if known else None


def learn_user_ids(run: LoadRun) -> int:
    """按名字（Load 007）从各个分片的名片里认出每个 Agent 的 Matrix 用户 ID。只等点名的那种叫醒方式
    一开始就要点名，等不到从收到的消息里学。名片是房间状态（网络 Agent 在 Matrix 上没设显示名），
    应用服务建的分片它自己就在房间里，读得到。"""
    for room_id in {agent.room_id for agent in run.agents}:
        request = Request(f"{MATRIX_API}/rooms/{quote(room_id, safe='')}/state")
        request.add_header("Authorization", f"Bearer {run.appservice_token}")
        try:
            with urlopen(request, timeout=30) as response:
                events = json.loads(response.read())
        except (OSError, HTTPException, ValueError) as error:
            print(f"  读不到 {room_id} 的房间状态（{type(error).__name__}），点名的对象只从收到的消息里学。")
            continue
        for event in events if isinstance(events, list) else []:
            learned = card_user_id(event)
            if learned is not None:
                run.ledger.learn(*learned)
    return sum(1 for agent in run.agents if run.ledger.user_id(agent.index) is not None)


def card_user_id(event: object) -> tuple[int, str] | None:
    """名片上写着 Agent 的名字和 Matrix 用户 ID；名字是 Load 007 这样的才算。"""
    if not isinstance(event, dict) or event.get("type") != STATUS_EVENT_TYPE:
        return None
    content = event.get("content")
    actor = content.get("actor") if isinstance(content, dict) else None
    agent = actor.get("agent") if isinstance(actor, dict) else None
    if not isinstance(agent, dict):
        return None
    name, user_id = agent.get("displayName"), agent.get("matrixUserId") or event.get("sender")
    match = LOAD_NAME.search(name) if isinstance(name, str) else None
    if match is None or not isinstance(user_id, str):
        return None
    return int(match.group("index")), user_id


# ---- 一档一档地量 ----


@dataclass
class LoadRun:
    meter: Meter
    appservice_token: str
    wake: str = WAKE_MODES[0]
    ledger: Ledger = field(default_factory=Ledger)
    growth: Growth = field(default_factory=Growth)
    agents: list[LoadAgent] = field(default_factory=list)
    stop: threading.Event = field(default_factory=threading.Event)
    waiters: list[threading.Thread] = field(default_factory=list)
    rng: random.Random = field(default_factory=lambda: random.Random(20261026))

    def grow_to(self, count: int) -> dict[str, object]:
        indexes = range(len(self.agents), count)
        with ThreadPoolExecutor(CREATE_WORKERS, thread_name_prefix="create") as pool:
            created = list(pool.map(lambda index: create_agent(index, self.growth), indexes))
        for agent in created:
            thread = threading.Thread(
                target=wait_loop, args=(agent, self), name=f"waiter-{agent.index}", daemon=True
            )
            thread.start()
            self.waiters.append(thread)
        self.agents.extend(created)
        time.sleep(SETTLE_SECONDS)
        return self.growth.summary(count)

    def switch_wake(self, wake: str) -> None:
        """换叫醒方式：在等的那一轮最多 10 秒就回来，下一轮按新的等。"""
        if wake != self.wake:
            self.wake = wake
            time.sleep(SETTLE_SECONDS)

    def measure(self, rate: float, window: float) -> dict[str, object]:
        self.ledger.begin(self.wake)
        with PeakMemory(self.meter) as peak:
            before = self.meter.cpu_seconds()
            started = time.monotonic()
            if rate > 0:
                speak(self.agents, self.ledger, rate=rate, window=window,
                      appservice_token=self.appservice_token, rng=self.rng)
            else:
                time.sleep(window)
            elapsed = time.monotonic() - started
            after = self.meter.cpu_seconds()
        if rate > 0:
            time.sleep(GRACE_SECONDS)
        cores = {name: round((after[name] - before[name]) / elapsed, 3) for name in before}
        cores["total"] = round(sum(value for name, value in cores.items() if name != "harness"), 3)
        room_sizes = Counter(agent.room_id for agent in self.agents)
        return {
            "agents": len(self.agents),
            # 只等不说的那一段谁也不叫醒，叫醒方式不相干。
            "wake": self.wake if rate > 0 else "idle",
            "messagesPerSecond": rate,
            "shards": len(room_sizes),
            "windowSeconds": round(elapsed, 1),
            "cores": cores,
            "peakMemoryMiB": peak.mebibytes(),
            **self.ledger.summary(room_sizes),
            "withinProductionBudget": within_budget(cores),
        }

    def close(self) -> None:
        self.stop.set()
        for thread in self.waiters:
            thread.join(timeout=60)


@dataclass
class Results:
    growth: list[dict[str, object]] = field(default_factory=list)
    rounds: list[dict[str, object]] = field(default_factory=list)


def run_rounds(
    run: LoadRun,
    counts: Iterable[int],
    *,
    rates: Sequence[float],
    wakes: Sequence[str],
    window: float,
    results: Results,
) -> None:
    """每一档：建够 N 个，先只等不说，再按每种叫醒方式、每种说话的快慢各量一段。"""
    for count in counts:
        print(f"建到 {count} 个网络 Agent……", flush=True)
        growth = run.grow_to(count)
        growth["knownUserIds"] = learn_user_ids(run)
        print(f"  建完：{growth}", flush=True)
        results.growth.append(growth)
        record(results, run.measure(0.0, IDLE_WINDOW_SECONDS))
        for wake in wakes:
            run.switch_wake(wake)
            for rate in rates:
                record(results, run.measure(rate, window))


def record(results: Results, result: dict[str, object]) -> None:
    print(summary_line(result), flush=True)
    results.rounds.append(result)


def summary_line(result: Mapping[str, object]) -> str:
    cores = result["cores"]
    latency = result["latencySeconds"] or {}
    assert isinstance(cores, dict) and isinstance(latency, dict)
    return (
        f"N={result['agents']:>3} {result['wake']:<7} M={result['messagesPerSecond']:<3} "
        f"分片={result['shards']} Synapse={cores['synapse']:.2f} PostgreSQL={cores['postgres']:.2f} "
        f"控制面={cores['controlPlane']:.2f} 合计={cores['total']:.2f} 核（工具 {cores['harness']:.2f}）；"
        f"收到 {result['deliveriesReceived']}/{result['deliveriesExpected']}，"
        f"p50={latency.get('p50', '-')} p95={latency.get('p95', '-')} 秒；"
        f"错误 {result['errors'] or '无'}；"
        f"{'撑得住' if result['withinProductionBudget'] else '生产撑不住'}"
    )


def start_control_plane(
    processes: v.ProcessStack, environment: Mapping[str, str], redactor: v.LogRedactor
) -> v.ManagedProcess:
    runtime = v.vertical_control_plane_environment(environment)
    runtime.update(PRODUCTION_LIKE)
    runtime.update(LIMIT_OVERRIDES)
    runtime["AGENT_ROOM_LOG_FILTER"] = CONTROL_PLANE_LOG_FILTER
    v.NETWORK_AGENT_STORE_ROOT.mkdir(parents=True, exist_ok=True)
    binary = v.ROOT / "target" / "release" / "agent-room-control-plane"
    control_plane = processes.start(v.ManagedProcess(
        name="control-plane", command=[str(binary)], environment=runtime,
        log_path=CONTROL_PLANE_LOG, redactor=redactor,
    ))
    v.wait_for_http("http://127.0.0.1:8090/health/live", control_plane, timeout_seconds=120)
    return control_plane


def parse_arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--agents", type=int, nargs="+", default=list(AGENT_COUNTS),
                        help="每一档建到几个网络 Agent，从小到大")
    parser.add_argument("--rates", type=float, nargs="+", default=list(MESSAGE_RATES),
                        help="每一档里每秒说几句（另外总有一段只等不说）")
    parser.add_argument("--wake", nargs="+", choices=WAKE_MODES, default=list(WAKE_MODES),
                        help="等消息的叫醒方式")
    parser.add_argument("--window", type=float, default=WINDOW_SECONDS, help="说话的那一段量多少秒")
    arguments = parser.parse_args(argv)
    if arguments.agents != sorted(arguments.agents) or min(arguments.agents) < 2:
        parser.error("--agents 要从小到大，每档至少 2 个")
    if min(arguments.rates) <= 0:
        parser.error("--rates 要大于 0")
    return arguments


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_arguments(argv)
    if not sys.platform.startswith("linux"):
        raise v.VerticalFailure("负载测量要在隔离的 Linux 机器上跑（读 cgroup 量 CPU）。")
    environment = v.prepare_environment()
    v.run_checked([v.executable("cargo"), "build", "--locked", "--release",
                   "-p", "agent-room-control-plane"])
    results = Results()
    try:
        with v.IsolatedInfrastructure():
            v.initialize_isolated_dependencies()
            v.seed_public_catalog()
            redactor = v.LogRedactor(environment)
            with v.ProcessStack() as processes:
                control_plane = start_control_plane(processes, environment, redactor)
                run = LoadRun(
                    meter=Meter.attach(control_plane.process.pid),
                    appservice_token=v.required_value(environment, "SYNAPSE_APPSERVICE_TOKEN"),
                )
                try:
                    run_rounds(run, arguments.agents, rates=arguments.rates, wakes=arguments.wake,
                               window=arguments.window, results=results)
                finally:
                    run.close()
    finally:
        # 中途失败也把量到的那几档留下来。
        write_report(results)
    return 0


def write_report(results: Results) -> None:
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps({
        "schemaVersion": 1,
        "waitQueries": WAIT_QUERIES,
        "budgetCores": {"synapse": SYNAPSE_CORE_BUDGET, "total": TOTAL_CORE_BUDGET},
        "growth": results.growth,
        "rounds": results.rounds,
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"结果写在 {REPORT.relative_to(v.ROOT)}。")


def run_cli(entry: Callable[[], int]) -> None:
    v.configure_console_encoding()
    try:
        raise SystemExit(entry())
    except v.VerticalFailure as error:
        print(f"负载测量失败：{error}", file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    run_cli(main)
