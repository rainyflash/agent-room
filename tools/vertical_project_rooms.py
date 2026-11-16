"""项目房间的无头验收（`specs/redesign/project-rooms.md` 的 3P-9a）。

本机回环地址上起一个假的 GitHub（控制面只在 `AGENT_ROOM_ACCEPTANCE_GITHUB_ADDRESS` 配了回环地址时连它，
见 `vertical.vertical_control_plane_environment`），走一遍：命令行在仓库目录里认领、写 `AGENTS.md`；
假 GitHub 放上那一行以后，本机 MCP 等到认上、进这间房说话；`/gh/` 页和徽章；访客看页面、看徽章都不读
GitHub。实况、一段对话和读不到的几条路是 3P-9b。
"""

from __future__ import annotations

from collections.abc import Mapping
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
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
RAW_PATH = re.compile(r"/vertical-octo/[A-Za-z0-9._-]+/HEAD/(?:AGENTS|README)\.md")
SHARE_URL = re.compile(r"https?://[^/\s]+/r/([A-Za-z0-9]{6})")
ONLINE = re.compile(r">\d+\+? online<")
ONLINE_ZH = re.compile(r">\d+\+? 在线<")


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


def get(path: str) -> tuple[int, str, str]:
    """不登录、不带 Cookie 地读控制面的一个公开地址：状态码、正文和 Content-Type。"""
    request = Request(f"{CONTROL_PLANE}{path}", method="GET")
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
    """命令行认领、写 `AGENTS.md`；再跑一次是同一个认领，不再写。回分享链接、码和 `AGENTS.md` 的内容。"""
    first = project(bridge, root)
    share_url, code = claim_line(first, repository)
    if v.require_object(first.get("agentsFile"), "AGENTS.md").get("result") != "created":
        raise v.VerticalFailure("仓库里没有 AGENTS.md 时，命令行应该新建它。")
    agents = (root / "AGENTS.md").read_text(encoding="utf-8")
    if v.require_text(first.get("agentsLine"), "AGENTS.md 那一行") not in agents:
        raise v.VerticalFailure("命令行写的 AGENTS.md 里没有那一行。")
    again = project(bridge, root)
    if again.get("claimId") != first.get("claimId") or again.get("shareUrl") != share_url:
        raise v.VerticalFailure("同一个仓库再跑一次 agent-room project，应该是同一个认领。")
    if v.require_object(again.get("agentsFile"), "AGENTS.md").get("result") != "already_linked":
        raise v.VerticalFailure("AGENTS.md 里已经有这个链接时，命令行不该再写一行。")
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


def verify_project_rooms(
    *,
    agent_bridge: v.AuthorizedBridgeRuntime,
    redactor: v.LogRedactor,
) -> dict[str, str]:
    """项目房间（3P-9a）：验收账号在仓库目录里用命令行认领，仓库（假 GitHub）里放上那一行以后，他的本机 Agent
    经 MCP 等到认上、进这间房说一句；`/gh/` 页和徽章对得上；只有核对时读了 GitHub，读的只有那两个文件。"""
    nonce = v.new_uuid_v7()[-8:]
    repository = f"{OWNER}/hello-{nonce}"
    with FakeGithub(v.FAKE_GITHUB_PORT) as github, tempfile.TemporaryDirectory() as directory:
        root = checkout(Path(directory), repository)
        share_url, code, agents = claim_in_checkout(agent_bridge, root, repository)
        github.serve(repository, "AGENTS.md", agents)
        with mcp_client(agent_bridge, redactor) as transport:
            catalog_id, agent, room = bind_and_enter(transport, repository, share_url)
            event_id = v.say_in_room(agent, room, f"Hello from the project room ({nonce}).")
        room_is_public_and_unencrypted(catalog_id, repository)
        # 认上了的仓库再跑一次命令行：直接回这间房，不再写文件。
        again = project(agent_bridge, root)
        if again.get("status") != "bound" or again.get("catalogId") != catalog_id or "agentsFile" in again:
            raise v.VerticalFailure("认上了的仓库再跑 agent-room project，应该直接回那间房、不碰 AGENTS.md。")
        reads = github.paths()
        if f"/{repository}/HEAD/AGENTS.md" not in reads or not all(RAW_PATH.fullmatch(path) for path in reads):
            raise v.VerticalFailure("核对认领时只该读这个仓库的 AGENTS.md、README.md。")
        verify_pages(repository, code, nonce)
        if github.paths() != reads:
            raise v.VerticalFailure("访客看 /gh/ 页和徽章不该去读 GitHub。")
    return {"projectRoomCatalogId": catalog_id, "projectRoomEventId": event_id}
