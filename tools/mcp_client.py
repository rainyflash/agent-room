"""面向真实纵向验收的最小 MCP stdio 客户端。"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
import json
from pathlib import Path
from queue import Empty, Queue
import subprocess
import threading
import time
from typing import Final, TextIO


JsonObject = dict[str, object]
LineSanitizer = Callable[[str], str]
PROTOCOL_VERSION: Final = "2025-11-25"


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
        names: list[str] = []
        for tool in tools:
            if not isinstance(tool, dict):
                raise McpClientFailure("MCP 工具定义必须是对象。")
            name = tool.get("name")
            if not isinstance(name, str) or not name:
                raise McpClientFailure("MCP 工具定义缺少名称。")
            names.append(name)
        return tuple(names)

    def call_tool(self, name: str, arguments: Mapping[str, object]) -> JsonObject:
        result = self.call_tool_result(name, arguments)
        if result.get("isError") is True:
            raise McpClientFailure(f"MCP 工具 {name} 返回失败。")
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


def _string_keyed_object(payload: Mapping[object, object], label: str) -> JsonObject:
    if not all(isinstance(name, str) for name in payload):
        raise McpClientFailure(f"{label} 只能包含字符串字段名。")
    return {str(name): value for name, value in payload.items()}
