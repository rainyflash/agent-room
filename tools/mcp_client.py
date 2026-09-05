"""面向真实纵向验收的最小 MCP stdio 客户端。"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
from dataclasses import dataclass
import json
from pathlib import Path
from queue import Empty, Queue
import re
import subprocess
import threading
import time
from typing import Final, TextIO


JsonObject = dict[str, object]
LineSanitizer = Callable[[str], str]
PROTOCOL_VERSION: Final = "2025-11-25"
STABLE_ERROR_CODE: Final = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")
SESSION_ID: Final = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
)
SESSION_SCOPED_TOOLS: Final = (
    "agent_room_get_self",
    "agent_room_list_previews",
    "agent_room_get_presence",
    "agent_room_open_content",
    "agent_room_publish_status",
    "agent_room_send_message",
    "agent_room_list_handoffs",
    "agent_room_consume_handoff",
    "agent_room_decline_handoff",
)
AGENT_ROOM_TOOLS: Final = (
    "agent_room_open_session", "agent_room_close_session", *SESSION_SCOPED_TOOLS
)


class McpClientFailure(RuntimeError):
    """表示 MCP 进程、协议或工具响应不符合验收契约。"""


class McpStdioClient(AbstractContextManager["McpStdioClient"]):
    """按请求串行化 MCP stdio 调用，并严格校验 JSON-RPC 响应。"""

    def __init__(
        self,
        *,
        command: Sequence[str],
        working_directory: Path,
        environment: Mapping[str, str],
        stderr_path: Path,
        sanitize_line: LineSanitizer,
        request_timeout_seconds: float = 30,
    ) -> None:
        if request_timeout_seconds <= 0:
            raise ValueError("MCP 请求超时必须大于零。")
        self._command = tuple(command)
        self._working_directory = working_directory
        self._environment = dict(environment)
        self._stderr_path = stderr_path
        self._sanitize_line = sanitize_line
        self._request_timeout_seconds = request_timeout_seconds
        self._responses: Queue[str | None] = Queue()
        self._process: subprocess.Popen[str] | None = None
        self._stdout_reader: threading.Thread | None = None
        self._stderr_reader: threading.Thread | None = None
        self._stderr_file: TextIO | None = None
        self._next_request_id = 1

    def __enter__(self) -> "McpStdioClient":
        self.start()
        self.initialize()
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        self.close()

    def start(self) -> None:
        if self._process is not None:
            raise McpClientFailure("MCP 客户端不能重复启动。")
        self._stderr_path.parent.mkdir(parents=True, exist_ok=True)
        self._stderr_file = self._stderr_path.open(
            "w", encoding="utf-8", newline="\n"
        )
        self._process = subprocess.Popen(
            self._command,
            cwd=self._working_directory,
            env=self._environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        self._stdout_reader = threading.Thread(
            target=self._read_stdout,
            name="agent-room-mcp-stdout",
            daemon=True,
        )
        self._stderr_reader = threading.Thread(
            target=self._read_stderr,
            name="agent-room-mcp-stderr",
            daemon=True,
        )
        self._stdout_reader.start()
        self._stderr_reader.start()

    def initialize(self) -> None:
        result = self.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "agent-room-vertical",
                    "version": "0.1.0",
                },
            },
        )
        protocol_version = result.get("protocolVersion")
        if not isinstance(protocol_version, str):
            raise McpClientFailure("MCP initialize 缺少协议版本。")
        self.notify("notifications/initialized", {})

    def list_tool_names(self) -> tuple[str, ...]:
        result = self.request("tools/list", {})
        tools = result.get("tools")
        if not isinstance(tools, list):
            raise McpClientFailure("MCP tools/list 缺少工具数组。")
        validate_session_tool_schemas(tools)
        names: list[str] = []
        for tool in tools:
            if not isinstance(tool, dict):
                raise McpClientFailure("MCP 工具定义必须是对象。")
            name = tool.get("name")
            if not isinstance(name, str) or not name:
                raise McpClientFailure("MCP 工具定义缺少名称。")
            names.append(name)
        return tuple(names)

    def open_session(self, *, session_key: str, display_name: str) -> "McpAgentSession":
        require_session_id(session_key, "sessionKey")
        if (
            not 1 <= len(display_name) <= 128
            or display_name.strip() != display_name
            or any(ord(character) < 32 or 127 <= ord(character) <= 159 for character in display_name)
        ):
            raise McpClientFailure("displayName 必须是 1–128 字符且不含边界空白或控制字符。")
        response = self.call_tool(
            "agent_room_open_session",
            {"sessionKey": session_key, "displayName": display_name},
        )
        summary = host_session_summary(response)
        if summary.get("state") not in {"starting", "ready"}:
            raise McpClientFailure("建立任务会话未返回 starting 或 ready 状态。")
        session_id = require_session_id(summary.get("sessionId"), "session.sessionId")
        return self.bind_session(session_id)

    def bind_session(self, session_id: str) -> "McpAgentSession":
        return McpAgentSession(self, require_session_id(session_id, "sessionId"))

    def call_tool(self, name: str, arguments: Mapping[str, object]) -> JsonObject:
        result = self.call_tool_result(name, arguments)
        if result.get("isError") is True:
            code = tool_failure_code(result)
            suffix = f"（错误码 {code}）" if code is not None else ""
            raise McpClientFailure(f"MCP 工具 {name} 返回失败{suffix}。")
        structured = result.get("structuredContent")
        if not isinstance(structured, dict):
            raise McpClientFailure(f"MCP 工具 {name} 缺少结构化响应。")
        return _string_keyed_object(structured, f"MCP 工具 {name} 的结构化响应")

    def call_tool_result(
        self, name: str, arguments: Mapping[str, object]
    ) -> JsonObject:
        """保留 MCP 错误结果，供故障恢复验收检查稳定错误码。"""
        return self.request(
            "tools/call",
            {"name": name, "arguments": dict(arguments)},
        )

    def request(self, method: str, params: Mapping[str, object]) -> JsonObject:
        request_id = self._next_request_id
        self._next_request_id += 1
        self._write(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": dict(params),
            }
        )
        deadline = time.monotonic() + self._request_timeout_seconds
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise McpClientFailure(f"MCP 请求 {method} 超时。")
            try:
                line = self._responses.get(timeout=remaining)
            except Empty as error:
                raise McpClientFailure(f"MCP 请求 {method} 超时。") from error
            if line is None:
                return_code = self._require_process().poll()
                raise McpClientFailure(
                    f"MCP 在响应 {method} 前退出，退出码 {return_code}。"
                )
            response = parse_json_object(line, "MCP JSON-RPC 响应")
            if response.get("id") != request_id:
                continue
            error_payload = response.get("error")
            if error_payload is not None:
                raise McpClientFailure(f"MCP 请求 {method} 返回 JSON-RPC 错误。")
            result = response.get("result")
            if not isinstance(result, dict):
                raise McpClientFailure(f"MCP 请求 {method} 缺少结果对象。")
            return _string_keyed_object(result, f"MCP 请求 {method} 的结果")

    def notify(self, method: str, params: Mapping[str, object]) -> None:
        self._write(
            {
                "jsonrpc": "2.0",
                "method": method,
                "params": dict(params),
            }
        )

    def close(self) -> None:
        process = self._process
        if process is None:
            return
        if process.stdin is not None:
            try:
                process.stdin.close()
            except OSError:
                pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        for reader in (self._stdout_reader, self._stderr_reader):
            if reader is not None:
                reader.join(timeout=3)
        if self._stderr_file is not None:
            self._stderr_file.close()
        self._process = None
        self._stdout_reader = None
        self._stderr_reader = None
        self._stderr_file = None

    def _write(self, payload: Mapping[str, object]) -> None:
        process = self._require_process()
        if process.poll() is not None or process.stdin is None:
            raise McpClientFailure("MCP 进程不可写。")
        try:
            process.stdin.write(
                json.dumps(dict(payload), ensure_ascii=False, separators=(",", ":"))
                + "\n"
            )
            process.stdin.flush()
        except (BrokenPipeError, OSError) as error:
            raise McpClientFailure("MCP stdio 写入失败。") from error

    def _read_stdout(self) -> None:
        process = self._require_process()
        if process.stdout is None:
            self._responses.put(None)
            return
        for line in process.stdout:
            self._responses.put(line)
        self._responses.put(None)

    def _read_stderr(self) -> None:
        process = self._require_process()
        stream = process.stderr
        destination = self._stderr_file
        if stream is None or destination is None:
            return
        for line in stream:
            destination.write(self._sanitize_line(line))
            destination.flush()

    def _require_process(self) -> subprocess.Popen[str]:
        if self._process is None:
            raise McpClientFailure("MCP 客户端尚未启动。")
        return self._process


def parse_json_object(text: str, label: str) -> JsonObject:
    try:
        payload: object = json.loads(text)
    except json.JSONDecodeError as error:
        raise McpClientFailure(f"{label} 不是有效 JSON。") from error
    if not isinstance(payload, dict):
        raise McpClientFailure(f"{label} 必须是对象。")
    return _string_keyed_object(payload, label)


def tool_failure_code(result: Mapping[str, object]) -> str | None:
    """仅提取可安全写入验收日志的稳定 MCP 错误码。"""
    structured = result.get("structuredContent")
    if not isinstance(structured, dict):
        return None
    code = structured.get("code")
    if structured.get("type") == "host_session":
        session = structured.get("session")
        code = session.get("errorCode") if isinstance(session, dict) else None
    if not isinstance(code, str) or STABLE_ERROR_CODE.fullmatch(code) is None:
        return None
    return code


@dataclass(frozen=True)
class McpAgentSession:
    """显式、不可变的会话绑定；多个绑定可共用 transport，绝不切换全局身份。"""

    transport: McpStdioClient
    session_id: str

    def __post_init__(self) -> None:
        require_session_id(self.session_id, "sessionId")

    def call_tool(self, name: str, arguments: Mapping[str, object]) -> JsonObject:
        return self.transport.call_tool(name, self._arguments(name, arguments))

    def call_tool_result(self, name: str, arguments: Mapping[str, object]) -> JsonObject:
        return self.transport.call_tool_result(name, self._arguments(name, arguments))

    def close(self) -> None:
        response = self.transport.call_tool(
            "agent_room_close_session", {"sessionId": self.session_id}
        )
        summary = host_session_summary(response)
        if summary.get("sessionId") != self.session_id or summary.get("state") != "closed":
            raise McpClientFailure("关闭任务会话没有确认原 sessionId 已 closed。")

    def _arguments(self, name: str, arguments: Mapping[str, object]) -> JsonObject:
        if name not in SESSION_SCOPED_TOOLS or "sessionId" in arguments:
            raise McpClientFailure("会话绑定只允许 Agent 工具，且不能覆盖 sessionId。")
        return {**arguments, "sessionId": self.session_id}


def require_session_id(value: object, label: str) -> str:
    if not isinstance(value, str) or SESSION_ID.fullmatch(value) is None:
        raise McpClientFailure(f"{label} 必须是规范小写 UUIDv7。")
    return value


def host_session_summary(response: Mapping[str, object]) -> JsonObject:
    summary = response.get("session")
    if response.get("type") != "host_session" or not isinstance(summary, dict):
        raise McpClientFailure("MCP 会话生命周期响应必须为 host_session。")
    return _string_keyed_object(summary, "MCP 任务会话摘要")


def validate_session_tool_schemas(tools: Sequence[object]) -> None:
    """发布门禁验证会话参数真实必填，不把声明了可选字段视为兼容。"""
    names: list[str] = []
    for tool in tools:
        if not isinstance(tool, dict) or not isinstance(tool.get("name"), str):
            raise McpClientFailure("MCP 工具定义缺少名称。")
        name = tool["name"]
        names.append(name)
        schema = tool.get("inputSchema")
        if not isinstance(schema, dict) or schema.get("type") != "object":
            raise McpClientFailure(f"MCP 工具 {name} 缺少对象输入 schema。")
        properties, required = schema.get("properties"), schema.get("required")
        if not isinstance(properties, dict) or not isinstance(required, list):
            raise McpClientFailure(f"MCP 工具 {name} 缺少必填会话参数。")
        field = "sessionKey" if name == "agent_room_open_session" else "sessionId"
        identifier = properties.get(field)
        if (
            field not in required
            or not isinstance(identifier, dict)
            or identifier.get("type") != "string"
            or identifier.get("minLength") != 36
            or identifier.get("maxLength") != 36
            or schema.get("additionalProperties") is not False
        ):
            raise McpClientFailure(f"MCP 工具 {name} 必须要求长度 36 的 {field} 并拒绝额外字段。")
        if name == "agent_room_open_session":
            display_name = properties.get("displayName")
            if (
                "sessionId" in properties
                or "displayName" not in required
                or not isinstance(display_name, dict)
                or display_name.get("type") != "string"
                or display_name.get("minLength") != 1
                or display_name.get("maxLength") != 128
            ):
                raise McpClientFailure("open_session 必须要求有界 displayName，不能接收 sessionId。")
    if len(names) != len(set(names)) or set(names) != set(AGENT_ROOM_TOOLS):
        raise McpClientFailure("MCP 工具集合必须包含两个会话工具和九个绑定会话的 Agent 工具。")


def _string_keyed_object(payload: Mapping[object, object], label: str) -> JsonObject:
    if not all(isinstance(name, str) for name in payload):
        raise McpClientFailure(f"{label} 只能包含字符串字段名。")
    return {str(name): value for name, value in payload.items()}
