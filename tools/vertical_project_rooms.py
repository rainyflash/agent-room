"""项目房间的无头验收（`specs/redesign/project-rooms.md` 的 3P-9a、3P-9b-1、3P-9b-2）。

本机回环地址上起一个假的 GitHub（控制面只在 `AGENT_ROOM_ACCEPTANCE_GITHUB_ADDRESS` 配了回环地址时连它，
见 `vertical.vertical_control_plane_environment`），走一遍：命令行在仓库目录里认领、写 `AGENTS.md`；
假 GitHub 放上那一行以后，本机 MCP 等到认上、进这间房说话；`/gh/` 页和徽章；访客看页面、看徽章都不读
GitHub（3P-9a）。接着是分享页的实况和读不到的几条路（3P-9b-1）：不登录看 `/r/<码>/live`，房主在网页上
关掉、再打开，改成要申请；私密的房间、加密的公开房间开不了（project-room-live.e2e.ts）。再打开以后、改成
要申请以前，房主在对话里分享一段，不登录打开 `/r/<码>/s/<段码>`；开头早于开实况的被拒；在房间的“分享”里
删掉一段，改成要申请（实况关了）以后留着的那段也打不开（3P-9b-2）。
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from html import unescape
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
from types import TracebackType
from urllib.error import HTTPError
from urllib.request import Request, urlopen

if __package__:
    from . import vertical as v
else:
    import vertical as v

CONTROL_PLANE = "http://127.0.0.1:8090"
OWNER = "vertical-octo"
AGENT_NAME = "Project helper"
# 名牌上的名字带四位编号；验收账号的显示名就是实况里写的主人。
LIVE_AGENT_NAME = re.compile(rf"{re.escape(AGENT_NAME)} #\d{{4}}")
AGENT_OWNER_NAME = "Local Developer"
RAW_PATH = re.compile(r"/vertical-octo/[A-Za-z0-9._-]+/HEAD/(?:AGENTS|README)\.md")
SHARE_URL = re.compile(r"https?://[^/\s]+/r/([A-Za-z0-9]{6})")
ONLINE = re.compile(r">\d+\+? online<")
ONLINE_ZH = re.compile(r">\d+\+? 在线<")
LIVE_SPEC = "project-room-live.e2e.ts"
# 实况那一轮和网页用例靠这几个文件对步子：网页关掉了实况、Agent 在关着时说过了、网页再打开了、
# 这边看到再打开以后的那句了（Agent 接着说完了要分享的那一段，文件里写着那几句）、网页分享了两段
# （文件里写着链接）、这边打开过了、网页删掉了一段、这边看到它打不开了、网页把房间改成要申请了。
LIVE_STEPS: tuple[tuple[str, Path], ...] = (
    ("OFF", v.VERTICAL_ROOT / "project-live-off.json"),
    ("SAID_OFF", v.VERTICAL_ROOT / "project-live-said-off.json"),
    ("ON", v.VERTICAL_ROOT / "project-live-on.json"),
    ("SEEN_ON", v.VERTICAL_ROOT / "project-live-seen-on.json"),
    ("SHARED", v.VERTICAL_ROOT / "project-live-shared.json"),
    ("SEEN_SHARED", v.VERTICAL_ROOT / "project-live-seen-shared.json"),
    ("DELETED", v.VERTICAL_ROOT / "project-live-deleted.json"),
    ("SEEN_DELETED", v.VERTICAL_ROOT / "project-live-seen-deleted.json"),
    ("REQUEST", v.VERTICAL_ROOT / "project-live-request.json"),
)
LIVE_STEP = dict(LIVE_STEPS)
# 不放实况以后，`/live` 最多 5 秒还按上次查到的回答（`RoomLiveWatch` 的 `ELIGIBILITY_TTL_MS`）。
LIVE_GONE_SECONDS = 30
LIVE_SHOWN_SECONDS = 90
# 一段对话那一页按两个码查到的在内存里留 5 秒（`RoomExcerptPageLookup`）：删掉、关了实况以后最多再放这么久。
EXCERPT_GONE_SECONDS = 30
# 网页给的一段对话的链接：`https://<裸域>/r/<分享码>/s/<段码>`。
EXCERPT_URL = re.compile(r"https?://[^/\s]+/r/([A-Za-z0-9]{6})/s/([A-Za-z0-9]{8})")
# 一段对话那一页（`share_page/excerpt.rs`）上的每一句和里面的名字、是谁的 Agent、正文。不公开的、只有附件的
# 那一行是 `share-live-text share-live-text--quiet`，不算正文。
EXCERPT_MESSAGE = re.compile(r'<li class="share-live-message">(.*?)</li>', re.DOTALL)
EXCERPT_NAME = re.compile(r'<span class="share-live-name">(.*?)</span>', re.DOTALL)
EXCERPT_KIND = re.compile(r'<span class="share-live-kind">(.*?)</span>', re.DOTALL)
EXCERPT_TEXT = re.compile(r'<p class="share-live-text">(.*?)</p>', re.DOTALL)
# 英文的“谁的 Agent”（`render.rs` 的 `agent_of`）。
AGENT_OF = re.compile(r"(.+)’s agent")


class FakeGithub:
    """照 raw.githubusercontent.com 的路径（`/owner/repo/HEAD/文件`）给文件，没有的回 404；记下每个请求的路径。"""

    def __init__(self, port: int) -> None:
        self._files: dict[str, bytes] = {}
        self._paths: list[str] = []
        self._lock = threading.Lock()
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802 - http.server 的命名
                body = fake.record(self.path)
                self.send_response(404 if body is None else 200)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Length", str(0 if body is None else len(body)))
                self.end_headers()
                if body is not None:
                    self.wfile.write(body)

            def log_message(self, format: str, *args: object) -> None:  # noqa: A002 - 照基类的签名
                return

        self._server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, name="fake-github", daemon=True)

    def __enter__(self) -> FakeGithub:
        self._thread.start()
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=10)

    def record(self, path: str) -> bytes | None:
        with self._lock:
            self._paths.append(path)
            return self._files.get(path)

    def serve(self, repository: str, file: str, content: str) -> None:
        with self._lock:
            self._files[f"/{repository}/HEAD/{file}"] = content.encode("utf-8")

    def paths(self) -> list[str]:
        with self._lock:
            return list(self._paths)


def get(path: str, *, language: str | None = None) -> tuple[int, str, str]:
    """不登录、不带 Cookie 地读控制面的一个公开地址：状态码、正文和 Content-Type。"""
    headers = {} if language is None else {"Accept-Language": language}
    request = Request(f"{CONTROL_PLANE}{path}", method="GET", headers=headers)
    try:
        with urlopen(request, timeout=30) as response:
            return response.status, response.read().decode("utf-8"), response.headers.get("Content-Type", "")
    except HTTPError as error:
        return error.code, error.read().decode("utf-8"), error.headers.get("Content-Type", "")


def checkout(directory: Path, repository: str) -> Path:
    """一个只有 `origin` 的 git 仓库，命令行从它认出 `owner/repo`。"""
    root = directory / "checkout"
    root.mkdir()
    for args in (("init", "--quiet"), ("remote", "add", "origin", f"https://github.com/{repository}.git")):
        completed = subprocess.run(("git", *args), cwd=root, capture_output=True, check=False)
        if completed.returncode != 0:
            raise v.VerticalFailure(f"git {args[0]} 失败，退出码 {completed.returncode}。")
    return root


def project(bridge: v.AuthorizedBridgeRuntime, directory: Path) -> dict[str, object]:
    """在仓库目录里跑 `agent-room project`（不带 --wait），回 `data`。"""
    completed = subprocess.run(
        (str(v.runtime_binary("agent-room")), "project"),
        cwd=directory,
        env=dict(bridge.environment),
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    try:
        output = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise v.VerticalFailure(f"agent-room project 没有回 JSON，退出码 {completed.returncode}。") from error
    if completed.returncode != 0 or output.get("ok") is not True:
        code = v.require_object(output.get("error"), "命令行的错误").get("code")
        raise v.VerticalFailure(f"agent-room project 失败：{code}。")
    return v.require_object(output.get("data"), "agent-room project 的回答")


def claim_line(data: Mapping[str, object], repository: str) -> tuple[str, str]:
    """等着的认领：那一行和徽章都指着同一个分享链接。回分享链接和它的码。"""
    share_url = v.require_text(data.get("shareUrl"), "分享链接")
    match = SHARE_URL.fullmatch(share_url)
    if match is None:
        raise v.VerticalFailure("认领给的分享链接不是 /r/<码>。")
    if data.get("repository") != repository or data.get("status") != "pending":
        raise v.VerticalFailure("命令行第一次认领应该是等着的、认的是这个仓库。")
    if share_url not in v.require_text(data.get("agentsLine"), "AGENTS.md 那一行"):
        raise v.VerticalFailure("AGENTS.md 那一行里没有这次认领的分享链接。")
    badge = v.require_text(data.get("readmeBadge"), "README 徽章")
    if f"/gh/{repository}/badge.svg" not in badge or share_url not in badge:
        raise v.VerticalFailure("README 徽章的图不是 /gh/ 地址，或者点进去不是这次认领的分享链接。")
    return share_url, match.group(1)


def claim_in_checkout(bridge: v.AuthorizedBridgeRuntime, root: Path, repository: str) -> tuple[str, str, str]:
    """命令行认领、在 `AGENTS.md` 里写一节；再跑一次是同一个认领，那一节不写第二遍。回分享链接、码和
    `AGENTS.md` 的内容。"""
    first = project(bridge, root)
    share_url, code = claim_line(first, repository)
    if v.require_object(first.get("agentsFile"), "AGENTS.md").get("result") != "created":
        raise v.VerticalFailure("仓库里没有 AGENTS.md 时，命令行应该新建它。")
    agents = (root / "AGENTS.md").read_text(encoding="utf-8")
    section = v.require_text(first.get("agentsSection"), "AGENTS.md 那一节")
    if section not in agents or share_url not in section:
        raise v.VerticalFailure("命令行写的 AGENTS.md 里没有带这次分享链接的那一节。")
    again = project(bridge, root)
    if again.get("claimId") != first.get("claimId") or again.get("shareUrl") != share_url:
        raise v.VerticalFailure("同一个仓库再跑一次 agent-room project，应该是同一个认领。")
    if v.require_object(again.get("agentsFile"), "AGENTS.md").get("result") != "unchanged":
        raise v.VerticalFailure("AGENTS.md 里已经是这一节时，命令行不该再改它。")
    if (root / "AGENTS.md").read_text(encoding="utf-8") != agents:
        raise v.VerticalFailure("再跑一次 agent-room project 把 AGENTS.md 改了。")
    return share_url, code, agents


def mcp_client(bridge: v.AuthorizedBridgeRuntime, redactor: v.LogRedactor) -> v.McpStdioClient:
    """和 `bridge_mcp_client` 一样，只是等得久一些：带 `wait` 的认领核对太快时要等下一分钟。"""
    return v.McpStdioClient(
        command=[str(v.runtime_binary("agent-room-mcp"))],
        working_directory=v.ROOT,
        environment=bridge.environment,
        stderr_path=v.LOG_ROOT / "project-rooms-session.log",
        sanitize_line=redactor.redact,
        request_timeout_seconds=240,
    )


def bind_and_enter(
    transport: v.McpStdioClient, repository: str, share_url: str
) -> tuple[str, v.McpAgentSession, str]:
    """本机 MCP 等到认上（第一次核对就读到假 GitHub 上的那一行），按分享链接进这间房。回目录 ID、会话和房间。"""
    bound = transport.call_tool("agent_room_claim_repository", {"repo": repository, "wait": True})
    if bound.get("status") != "bound" or bound.get("shareUrl") != share_url:
        raise v.VerticalFailure("仓库里放上那一行以后，带 wait 的认领应该认上，分享链接还是那一个。")
    catalog_id = v.require_text(bound.get("catalogId"), "认上的房间")
    if "agent_room_join" not in v.require_text(bound.get("next"), "认上以后的下一步"):
        raise v.VerticalFailure("认上以后应该告诉 Agent 用 agent_room_join 进这间房。")
    entered = transport.call_tool("agent_room_join", {"room": share_url, "displayName": AGENT_NAME})
    if v.require_object(entered.get("room"), "进的房间").get("catalogId") != catalog_id:
        raise v.VerticalFailure("按分享链接进的不是认上的那间房。")
    agent = transport.bind_session(v.require_text(entered.get("sessionId"), "本机 Agent 的会话"))
    room = v.require_text(v.wait_for_session_identity(agent, timeout_seconds=180).get("matrixRoomId"), "房间")
    return catalog_id, agent, room


def room_is_public_and_unencrypted(catalog_id: str, repository: str) -> None:
    """认上以后建的房间：名字是 owner/repo，公开、不加密，绑着这个仓库。"""
    if not re.fullmatch(r"[0-9a-f-]{36}", catalog_id):
        raise v.VerticalFailure("认上的房间 ID 不是 UUID。")
    row = v.compose_psql(
        "SELECT catalog.name, catalog.end_to_end_encrypted, COALESCE(listing.listed, false),"
        " binding.repository_owner_lower || '/' || binding.repository_name_lower"
        " FROM agent_room.room_catalog_entry catalog"
        " JOIN agent_room.room_repository_binding binding ON binding.catalog_entry_id = catalog.id"
        " LEFT JOIN agent_room.private_room_listing listing ON listing.catalog_entry_id = catalog.id"
        f" WHERE catalog.id = '{catalog_id}'"
    ).strip()
    if row != f"{repository}|f|t|{repository.lower()}":
        raise v.VerticalFailure("认上以后建的房间应该叫 owner/repo、公开、不加密、绑着这个仓库。")


def wait_for_badge(path: str, pattern: re.Pattern[str]) -> str:
    """徽章算出来的数在内存里留 60 秒：等到它写着在线数（最多 3 分钟）。"""
    deadline = time.monotonic() + 180
    while True:
        status, body, content_type = get(path)
        if status == 200 and content_type.startswith("image/svg+xml") and pattern.search(body):
            return body
        if time.monotonic() >= deadline:
            raise v.VerticalFailure(f"3 分钟内徽章 {path} 没有写着在线数（最后一次 {status}）。")
        time.sleep(5)


def verify_pages(repository: str, code: str, nonce: str) -> None:
    """`/gh/` 页：绑着的出分享页（大小写不论），没绑的出“还没人认领”（404），名字不合的 404；徽章同理。"""
    owner, name = repository.split("/")
    for path in (f"/gh/{repository}", f"/gh/{owner.upper()}/{name}"):
        status, body, _ = get(path)
        if status != 200 or repository not in body or f"/r/{code}" not in body:
            raise v.VerticalFailure(f"{path} 应该是绑上的那间房的分享页（{status}）。")
    wait_for_badge(f"/gh/{repository}/badge.svg", ONLINE)
    wait_for_badge(f"/gh/{repository}/badge.svg?lang=zh-CN", ONLINE_ZH)
    nobody = f"{OWNER}/nobody-{nonce}"
    status, body, _ = get(f"/gh/{nobody}")
    if status != 404 or f"nobody-{nonce}" not in body:
        raise v.VerticalFailure("没人认领的仓库应该出“还没人认领”，回 404。")
    for path, text in ((f"/gh/{nobody}/badge.svg", "no room"), (f"/gh/{nobody}/badge.svg?lang=zh-CN", "没有房间")):
        status, body, _ = get(path)
        if status != 200 or f">{text}<" not in body:
            raise v.VerticalFailure(f"没人认领的仓库的徽章应该写“{text}”（{status}）。")
    status, _, _ = get("/gh/-octo/hello")
    if status != 404:
        raise v.VerticalFailure("名字不合 GitHub 规矩的 /gh/ 地址应该 404。")


@dataclass(frozen=True)
class LiveLine:
    """实况里的一句和说这句的人。"""

    name: str
    kind: str
    owner: str | None
    text: str


@dataclass(frozen=True)
class ProjectRoom:
    """认上以后建的那间房：目录 ID、分享码、绑的仓库和 Agent 在里面说话的 Matrix 房间。"""

    catalog_id: str
    code: str
    repository: str
    room_id: str


def live_lines(snapshot: Mapping[str, object]) -> list[LiveLine]:
    """`/r/<码>/live` 的回答里的每一句，配上名单里按编号找到的说话人。不公开的那几句正文是空的。"""
    if snapshot.get("schemaVersion") != 1:
        raise v.VerticalFailure("实况的回答不是第 1 版。")
    people: dict[str, dict[str, object]] = {}
    for item in v.require_list(snapshot.get("participants")):
        person = v.require_object(item, "实况里说话的人")
        people[v.require_text(person.get("key"), "说话的人的编号")] = person
    lines = []
    for item in v.require_list(snapshot.get("messages")):
        message = v.require_object(item, "实况里的一句")
        author = people.get(v.require_text(message.get("author"), "这一句是谁说的"))
        if author is None:
            raise v.VerticalFailure("实况里有一句的说话人不在名单里。")
        owner, text = author.get("owner"), message.get("text")
        lines.append(
            LiveLine(
                name=v.require_text(author.get("name"), "说话的人的名字"),
                kind=v.require_text(author.get("kind"), "说话的人是人还是 Agent"),
                owner=owner if isinstance(owner, str) else None,
                text=text if isinstance(text, str) else "",
            )
        )
    return lines


def require_said_by_agent(lines: list[LiveLine], text: str) -> None:
    """那一句在实况里，是验收账号的本机 Agent 说的：名字带编号、是本机 Agent、写着主人。"""
    said = [line for line in lines if line.text == text]
    if len(said) != 1:
        raise v.VerticalFailure(f"实况里该有一句“{text}”，有 {len(said)} 句。")
    line = said[0]
    if LIVE_AGENT_NAME.fullmatch(line.name) is None or line.kind != "agent":
        raise v.VerticalFailure(f"实况里那一句该是本机 Agent“{AGENT_NAME} #编号”说的，写的是 {line.kind}。")
    if line.owner != AGENT_OWNER_NAME:
        raise v.VerticalFailure("实况里那一句该写着 Agent 的主人是验收账号。")


def has_live_panel(html: str, code: str) -> bool:
    """分享页（`/r/<码>`、`/gh/owner/repo`）上有没有实况那一块（`render.rs` 的 `live_panel`）。"""
    return f'data-live="/r/{code}/live"' in html


def read_live(code: str) -> tuple[int, list[LiveLine]]:
    """不登录读一次实况：状态码和里面的话（不是 200 时没有话）。"""
    status, body, content_type = get(f"/r/{code}/live")
    if status != 200:
        return status, []
    if not content_type.startswith("application/json"):
        raise v.VerticalFailure(f"实况的回答不是 JSON：{content_type}。")
    return status, live_lines(v.require_object(json.loads(body), "实况的回答"))


def wait_for_live(code: str, text: str) -> list[LiveLine]:
    """等到实况里出现这一句：快照有人看才每 3 秒重读，刚打开时还要等 5 秒的“不放”过期。"""
    deadline = time.monotonic() + LIVE_SHOWN_SECONDS
    while True:
        status, lines = read_live(code)
        if any(line.text == text for line in lines):
            return lines
        if time.monotonic() >= deadline:
            raise v.VerticalFailure(f"{LIVE_SHOWN_SECONDS} 秒内实况里没有“{text}”（最后一次 {status}）。")
        time.sleep(2)


def wait_until_not_live(code: str) -> None:
    """等到 `/r/<码>/live` 回 404 `room_live.unavailable`：查到的“放”最多再用 5 秒。"""
    deadline = time.monotonic() + LIVE_GONE_SECONDS
    while True:
        status, body, _ = get(f"/r/{code}/live")
        if status == 404:
            if v.require_object(json.loads(body), "实况的 404").get("code") != "room_live.unavailable":
                raise v.VerticalFailure("不放实况时 /live 该回 room_live.unavailable。")
            return
        if time.monotonic() >= deadline:
            raise v.VerticalFailure(f"{LIVE_GONE_SECONDS} 秒内 /r/<码>/live 还没回 404（最后一次 {status}）。")
        time.sleep(1)


def require_repository_page(project_room: ProjectRoom, *, live: bool) -> None:
    """`/gh/owner/repo` 还是这间房的分享页；放着实况时有那一块，不放时没有。"""
    status, body, _ = get(f"/gh/{project_room.repository}")
    if status != 200 or f"/r/{project_room.code}" not in body:
        raise v.VerticalFailure(f"/gh/ 页该是这间房的分享页（{status}）。")
    if has_live_panel(body, project_room.code) != live:
        state = "该有" if live else "不该有"
        raise v.VerticalFailure(f"/gh/ 页上{state}实况那一块。")


@dataclass(frozen=True)
class Said:
    """本机 Agent 在房间里说的一句：事件 ID 和正文。"""

    event_id: str
    text: str

    def payload(self) -> dict[str, str]:
        return {"eventId": self.event_id, "text": self.text}


def write_step(path: Path, payload: Mapping[str, object] | None = None) -> None:
    """写下这一步：先写旁边的临时文件再换名，网页那边看到文件时内容已经写全了。"""
    staging = path.with_name(f"{path.name}.tmp")
    staging.write_text(json.dumps(dict(payload or {}), ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(staging, path)


def excerpt_path(url: str, code: str) -> str:
    """网页给的一段对话的链接换成控制面上的地址（控制面就是裸域上 `/r/*` 转去的地方）；得在这间房的分享链接下面。"""
    match = EXCERPT_URL.fullmatch(url)
    if match is None:
        raise v.VerticalFailure("一段对话的链接不是 /r/<码>/s/<段码>。")
    if match.group(1) != code:
        raise v.VerticalFailure("一段对话的链接不在这间房的分享链接下面。")
    return f"/r/{match.group(1)}/s/{match.group(2)}"


def excerpt_author(label: str | None) -> tuple[str, str | None]:
    """名字旁边那一小块（英文）换成实况里的说法：人没有这一块，网络 Agent、没写主人的 Agent、谁的 Agent。"""
    if label is None:
        return "person", None
    if label == "Network agent":
        return "networkAgent", None
    if label == "Agent":
        return "agent", None
    match = AGENT_OF.fullmatch(label)
    if match is None:
        raise v.VerticalFailure("一段对话那一页上名字旁边写的不是“谁的 Agent”。")
    return "agent", match.group(1)


def excerpt_lines(html: str) -> list[LiveLine]:
    """一段对话那一页（英文）上的每一句：名字、是谁的 Agent、正文（不公开的、只有附件的没有正文）。"""
    lines = []
    for item in EXCERPT_MESSAGE.findall(html):
        name = EXCERPT_NAME.search(item)
        if name is None:
            raise v.VerticalFailure("一段对话那一页上有一句没写名字。")
        kind = EXCERPT_KIND.search(item)
        kind_name, owner = excerpt_author(None if kind is None else unescape(kind.group(1)))
        text = EXCERPT_TEXT.search(item)
        lines.append(
            LiveLine(
                name=unescape(name.group(1)),
                kind=kind_name,
                owner=owner,
                text="" if text is None else unescape(text.group(1)),
            )
        )
    return lines


def require_excerpt_page(html: str, project_room: ProjectRoom, stretch: Sequence[Said]) -> None:
    """一段对话那一页：房间名，“A conversation · N messages”，正好是那几句、照先后，每句都是验收账号的本机
    Agent 说的（名字带编号、写着主人），下面“去看这间房”回这间房的分享页。"""
    if f"<h1>{project_room.repository}</h1>" not in html:
        raise v.VerticalFailure("一段对话那一页上没有房间名。")
    count = "1 message" if len(stretch) == 1 else f"{len(stretch)} messages"
    if f"A conversation · {count} · " not in html:
        raise v.VerticalFailure(f"一段对话那一页上该写着“{count}”。")
    if f'/r/{project_room.code}"' not in html:
        raise v.VerticalFailure("一段对话那一页上没有回这间房分享页的链接。")
    lines = excerpt_lines(html)
    if [line.text for line in lines] != [said.text for said in stretch]:
        raise v.VerticalFailure("一段对话那一页上该正好是分享的那几句，照先后，前后的都不在。")
    for said in stretch:
        require_said_by_agent(lines, said.text)


def read_excerpt(path: str) -> tuple[int, str]:
    """不登录打开一段对话那一页（英文）：状态码和 HTML。"""
    status, body, content_type = get(path, language="en")
    if not content_type.startswith("text/html"):
        raise v.VerticalFailure(f"一段对话那一页回的不是 HTML（{status}）：{content_type}。")
    return status, body


def require_excerpt_shown(path: str, project_room: ProjectRoom, stretch: Sequence[Said]) -> None:
    status, body = read_excerpt(path)
    if status != 200:
        raise v.VerticalFailure(f"分享的一段该打得开，回了 {status}。")
    require_excerpt_page(body, project_room, stretch)


def wait_for_excerpt_gone(path: str, unknown: str) -> None:
    """等到这一段那一页和没有的段码回一模一样的 404：按两个码查到的最多再用 5 秒。"""
    deadline = time.monotonic() + EXCERPT_GONE_SECONDS
    while True:
        status, body = read_excerpt(path)
        if status == 404:
            if body != unknown:
                raise v.VerticalFailure("打不开的一段该和没有的段码回同一页 404，不说是哪一种。")
            return
        # 429 是这个来源一分钟里打开得太多（和分享页算在一起），等一等接着看。
        if status not in (200, 429):
            raise v.VerticalFailure(f"等这一段打不开时回了 {status}。")
        if time.monotonic() >= deadline:
            raise v.VerticalFailure(f"{EXCERPT_GONE_SECONDS} 秒内这一段还打得开。")
        time.sleep(2)


def say_stretch(agent: v.McpAgentSession, project_room: ProjectRoom, opening: Said, nonce: str) -> list[Said]:
    """要分享的那一段：再打开实况以后那句，Agent 接着再说两句；等到最后一句也在实况里。"""
    stretch = [opening]
    for text in (f"Second line of the shared stretch ({nonce}).", f"Last line of the shared stretch ({nonce})."):
        stretch.append(Said(event_id=v.say_in_room(agent, project_room.room_id, text), text=text))
    lines = wait_for_live(project_room.code, stretch[-1].text)
    for said in stretch:
        require_said_by_agent(lines, said.text)
    return stretch


def verify_excerpts(
    project_room: ProjectRoom, browser: v.BrowserRun, stretch: Sequence[Said], nonce: str
) -> tuple[str, str]:
    """一段对话（3P-9b-2）：网页在对话里选了这一段分享，又经接口把最后一句单独分享（开头早于开实况的那次被拒，
    网页那边查）。不登录打开两段都是那几句；网页在房间的“分享”里删掉第一段：它和没有的段码回同一页 404，
    第二段还在。回第二段的地址和没有的段码那一页，改成要申请以后接着看。"""
    shared = v.wait_for_browser_step(
        LIVE_STEP["SHARED"], browser, timeout_seconds=600, failure="网页 10 分钟内没在对话里分享一段。"
    )
    shown = excerpt_path(v.require_text(shared.get("shown"), "对话里分享的一段"), project_room.code)
    kept = excerpt_path(v.require_text(shared.get("kept"), "经接口分享的一段"), project_room.code)
    require_excerpt_shown(shown, project_room, stretch)
    require_excerpt_shown(kept, project_room, stretch[-1:])
    status, unknown = read_excerpt(f"/r/{project_room.code}/s/Qq{nonce[-6:]}")
    if status != 404:
        raise v.VerticalFailure(f"没有的段码该回 404，回了 {status}。")
    write_step(LIVE_STEP["SEEN_SHARED"])

    v.wait_for_browser_step(
        LIVE_STEP["DELETED"], browser, timeout_seconds=300, failure="网页 5 分钟内没在房间的“分享”里删掉那一段。"
    )
    wait_for_excerpt_gone(shown, unknown)
    require_excerpt_shown(kept, project_room, stretch[-1:])
    write_step(LIVE_STEP["SEEN_DELETED"])
    return kept, unknown


def live_browser_environment(
    environment: Mapping[str, str], *, project_catalog: str, former_public_room: str
) -> dict[str, str]:
    """网页用例的环境：验收账号（项目房间的房主）、两间房和对步子的文件。"""
    playwright_environment = os.environ.copy()
    playwright_environment.update(
        {
            "AGENT_ROOM_E2E_USERNAME": "developer",
            "AGENT_ROOM_E2E_PASSWORD": v.required_value(environment, "SEED_ADMIN_PASSWORD"),
            "AGENT_ROOM_VERTICAL_PROJECT_LIVE_ROOM": project_catalog,
            "AGENT_ROOM_VERTICAL_PROJECT_LIVE_FORMER_PUBLIC_ROOM": former_public_room,
            **{f"AGENT_ROOM_VERTICAL_PROJECT_LIVE_{name}": str(path) for name, path in LIVE_STEPS},
        }
    )
    return playwright_environment


def verify_live_view(
    project_room: ProjectRoom,
    agent: v.McpAgentSession,
    *,
    first_line: str,
    nonce: str,
    environment: Mapping[str, str],
    former_public_room: str,
) -> str:
    """分享页放实况（3P-9b-1）：项目房间建好就开着，不登录在 `/r/<码>/live` 看得到 Agent 刚才那句（名字、
    主人），`/gh/` 页上有那一块。房主在网页上关掉：5 秒多一点就 404、那一块没了；关着时 Agent 说一句。再打开：
    之后说的那句在，打开之前的两句都不在。Agent 接着说完一段，网页分享、删掉（3P-9b-2，`verify_excerpts`）。
    房主改成要申请：实况自己关了、开不了（网页那边查），这边又是 404，留着的那一段也打不开。网页那边接着看
    私密的房间、加密的公开房间开不了。回再打开以后那一句的事件 ID。"""
    for _, path in LIVE_STEPS:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.unlink(missing_ok=True)
    require_said_by_agent(wait_for_live(project_room.code, first_line), first_line)
    require_repository_page(project_room, live=True)
    browser = v.BrowserRun(
        LIVE_SPEC,
        live_browser_environment(
            environment, project_catalog=project_room.catalog_id, former_public_room=former_public_room
        ),
    )
    v.wait_for_browser_step(
        LIVE_STEP["OFF"], browser, timeout_seconds=600, failure="网页 10 分钟内没在房间设置里关掉实况。"
    )
    wait_until_not_live(project_room.code)
    require_repository_page(project_room, live=False)
    said_off = f"Said while the live view was off ({nonce})."
    said_off_event_id = v.say_in_room(agent, project_room.room_id, said_off)
    write_step(LIVE_STEP["SAID_OFF"])

    v.wait_for_browser_step(
        LIVE_STEP["ON"], browser, timeout_seconds=300, failure="网页 5 分钟内没把实况再打开。"
    )
    said_on = f"Said after the live view was turned back on ({nonce})."
    event_id = v.say_in_room(agent, project_room.room_id, said_on)
    lines = wait_for_live(project_room.code, said_on)
    require_said_by_agent(lines, said_on)
    if {first_line, said_off} & {line.text for line in lines}:
        raise v.VerticalFailure("再打开实况以后，打开之前说的话（关之前的、关着时的）不该在实况里。")
    require_repository_page(project_room, live=True)
    stretch = say_stretch(agent, project_room, Said(event_id=event_id, text=said_on), nonce)
    # 网页从再打开以后那句选到最后一句；关着时说的那句没有“分享这一段”，经接口分享也被拒。
    write_step(
        LIVE_STEP["SEEN_ON"],
        {
            "before": Said(event_id=said_off_event_id, text=said_off).payload(),
            "first": stretch[0].payload(),
            "last": stretch[-1].payload(),
            "count": len(stretch),
        },
    )
    kept, unknown = verify_excerpts(project_room, browser, stretch, nonce)

    v.wait_for_browser_step(
        LIVE_STEP["REQUEST"], browser, timeout_seconds=300, failure="网页 5 分钟内没把项目房间改成要申请。"
    )
    # 网页接着把房间改回直接进：实况不会自己回来，下面两样改回以后也一样。实况一关，分享过的段跟着删了。
    wait_until_not_live(project_room.code)
    wait_for_excerpt_gone(kept, unknown)
    require_repository_page(project_room, live=False)
    browser.finish(timeout_seconds=600)
    return event_id


def verify_project_rooms(
    *,
    agent_bridge: v.AuthorizedBridgeRuntime,
    environment: Mapping[str, str],
    former_public_room: str,
    redactor: v.LogRedactor,
) -> dict[str, str]:
    """项目房间（3P-9a）：验收账号在仓库目录里用命令行认领，仓库（假 GitHub）里放上那一行以后，他的本机 Agent
    经 MCP 等到认上、进这间房说一句；`/gh/` 页和徽章对得上；只有核对时读了 GitHub，读的只有那两个文件。
    接着是分享页的实况和读不到的几条路（3P-9b-1，`verify_live_view`），中间分享一段对话（3P-9b-2）：
    `former_public_room` 是公开房间那一轮验收账号建的、最后改回私密的房间。"""
    nonce = v.new_uuid_v7()[-8:]
    repository = f"{OWNER}/hello-{nonce}"
    first_line = f"Hello from the project room ({nonce})."
    with FakeGithub(v.FAKE_GITHUB_PORT) as github, tempfile.TemporaryDirectory() as directory:
        root = checkout(Path(directory), repository)
        share_url, code, agents = claim_in_checkout(agent_bridge, root, repository)
        github.serve(repository, "AGENTS.md", agents)
        # 会话一直开到实况那一段做完：Agent 要在关着时、再打开以后各说一句。
        with mcp_client(agent_bridge, redactor) as transport:
            catalog_id, agent, room = bind_and_enter(transport, repository, share_url)
            event_id = v.say_in_room(agent, room, first_line)
            room_is_public_and_unencrypted(catalog_id, repository)
            # 认上了的仓库再跑一次命令行：直接回这间房，那一节已经在了，不再改文件。
            again = project(agent_bridge, root)
            if (
                again.get("status") != "bound"
                or again.get("catalogId") != catalog_id
                or v.require_object(again.get("agentsFile"), "AGENTS.md").get("result") != "unchanged"
                or (root / "AGENTS.md").read_text(encoding="utf-8") != agents
            ):
                raise v.VerticalFailure("认上了的仓库再跑 agent-room project，应该直接回那间房、不改 AGENTS.md。")
            reads = github.paths()
            if f"/{repository}/HEAD/AGENTS.md" not in reads or not all(RAW_PATH.fullmatch(path) for path in reads):
                raise v.VerticalFailure("核对认领时只该读这个仓库的 AGENTS.md、README.md。")
            verify_pages(repository, code, nonce)
            live_event_id = verify_live_view(
                ProjectRoom(catalog_id=catalog_id, code=code, repository=repository, room_id=room),
                agent,
                first_line=first_line,
                nonce=nonce,
                environment=environment,
                former_public_room=former_public_room,
            )
        if github.paths() != reads:
            raise v.VerticalFailure("访客看 /gh/ 页、徽章、实况和一段对话不该去读 GitHub。")
    return {
        "projectRoomCatalogId": catalog_id,
        "projectRoomEventId": event_id,
        "projectRoomLiveEventId": live_event_id,
    }
