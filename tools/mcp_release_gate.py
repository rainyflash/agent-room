#!/usr/bin/env python3
"""MCP 发版门禁：核对 Rust 里声明的工具和发行风险标注，并用发行构建的 agent-room-mcp 实跑 stdio 协议。

门禁只看通用的 agent-room-mcp，不绑定任何 Agent 应用：每个支持 MCP 的工具连的都是这同一个程序。
validate 只读源码，适合在占用原生构建资源之前跑；smoke 启动真实二进制，不连用户的 Bridge。
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Sequence
import uuid

if __package__:
    from .mcp_client import tool_failure_code, validate_session_tool_schemas
else:
    from mcp_client import tool_failure_code, validate_session_tool_schemas


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
MCP_SERVER_SOURCE = (
    REPOSITORY_ROOT / "apps" / "agent-room-mcp" / "src" / "agent_room" / "server.rs"
)
BINARY_BASENAME = "agent-room-mcp"
# 每个工具的 (readOnlyHint, destructiveHint, idempotentHint, openWorldHint)。
# 新增 MCP 工具时同步改这里、tools/mcp_client.py 和 server.rs 里的工具集合测试。
EXPECTED_TOOL_ANNOTATIONS = {
    "agent_room_list_rooms": (True, False, True, True),
    "agent_room_join": (False, False, True, True),
    "agent_room_open_session": (False, False, True, True),
    "agent_room_register_reception": (False, False, True, False),
    "agent_room_close_session": (False, False, True, True),
    "agent_room_get_self": (True, False, True, False),
    "agent_room_matrix_security": (False, False, False, True),
    "agent_room_list_previews": (True, False, True, True),
    "agent_room_wait_for_messages": (True, False, True, True),
    "agent_room_get_presence": (True, False, True, True),
    "agent_room_open_content": (True, False, True, True),
    "agent_room_publish_status": (False, False, True, True),
    "agent_room_send_message": (False, False, False, True),
    "agent_room_list_handoffs": (True, False, True, True),
    "agent_room_consume_handoff": (False, True, False, True),
    "agent_room_decline_handoff": (False, True, False, True),
}
TOOL_ATTRIBUTE_PATTERN = re.compile(r"#\[\s*tool\s*\(")
TOOL_NAME_PATTERN = re.compile(r'#\[\s*tool\s*\(\s*name\s*=\s*"([a-z0-9_]+)"')


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=("validate", "smoke"),
        help="validate 只核对源码里的工具声明；smoke 另外实跑 MCP 二进制的 stdio 协议",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        help="smoke 使用的 MCP 二进制；省略时构建当前平台的 release 版本",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> None:
    args = parse_args(argv)
    validate_source()
    if args.command == "validate":
        print(f"MCP 工具声明与发行风险标注一致：{MCP_SERVER_SOURCE}")
        return
    binary = resolve_binary(args.binary)
    smoke_test_mcp(binary)
    print(f"MCP 真实协议门禁通过：{binary}")


def validate_source() -> None:
    declared_tools = declared_mcp_tools()
    annotation_tools = tuple(EXPECTED_TOOL_ANNOTATIONS)
    if set(declared_tools) != set(annotation_tools):
        raise RuntimeError(
            "MCP Rust 工具声明与发行风险标注不一致："
            f"缺少标注 {set(declared_tools) - set(annotation_tools)}，"
            f"多余标注 {set(annotation_tools) - set(declared_tools)}"
        )


def declared_mcp_tools() -> tuple[str, ...]:
    source = MCP_SERVER_SOURCE.read_text(encoding="utf-8")
    tools = tuple(TOOL_NAME_PATTERN.findall(source))
    if not tools:
        raise RuntimeError("MCP Rust 源码未声明任何工具")
    if len(TOOL_ATTRIBUTE_PATTERN.findall(source)) != len(tools):
        raise RuntimeError("每个 MCP Rust 工具都必须显式声明稳定名称")
    if len(tools) != len(set(tools)):
        raise RuntimeError("MCP Rust 源码包含重复工具名称")
    if not all(name.startswith("agent_room_") for name in tools):
        raise RuntimeError("MCP Rust 源码包含非 agent_room 命名空间工具")
    return tools


def resolve_binary(explicit: Path | None) -> Path:
    if explicit is not None:
        binary = explicit.expanduser().resolve()
    else:
        subprocess.run(
            ["cargo", "build", "--release", "--locked", "-p", "agent-room-mcp"],
            cwd=REPOSITORY_ROOT,
            check=True,
        )
        suffix = ".exe" if os.name == "nt" else ""
        binary = REPOSITORY_ROOT / "target" / "release" / f"{BINARY_BASENAME}{suffix}"
    if not binary.is_file():
        raise RuntimeError(f"MCP 二进制不存在：{binary}")
    return binary


def smoke_requests() -> list[dict[str, object]]:
    return [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "agent-room-release-gate", "version": "0.1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "agent_room_get_self", "arguments": {}},
        },
        {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "agent_room_get_self",
                "arguments": {"sessionId": "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a60"},
            },
        },
        {
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "agent_room_open_session", "arguments": {}},
        },
        {
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "agent_room_close_session", "arguments": {}},
        },
    ]


def smoke_test_mcp(binary: Path, working_directory: Path | None = None) -> None:
    wire_input = "".join(
        f"{json.dumps(request, separators=(',', ':'))}\n" for request in smoke_requests()
    )
    try:
        # 门禁不连接用户的 Bridge，也不通过 open_session 注册远端 Agent：
        # 数据目录和安全存储命名空间都是一次性的。
        with tempfile.TemporaryDirectory(prefix="agent-room-mcp-gate-") as temporary:
            environment = os.environ.copy()
            environment.update(
                {
                    "AGENT_ROOM_BRIDGE_DATA_DIR": temporary,
                    "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE": (
                        f"dev.agent-room.smoke.{uuid.uuid4().hex}"
                    ),
                }
            )
            completed = subprocess.run(
                [str(binary)],
                cwd=working_directory or temporary,
                env=environment,
                input=wire_input,
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=20,
                check=False,
            )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("MCP 冒烟测试超时") from error
    if completed.returncode != 0:
        raise RuntimeError(
            f"MCP 冒烟测试进程失败：{completed.stderr.strip() or completed.returncode}"
        )
    validate_smoke_responses(parse_json_lines(completed.stdout))


def validate_smoke_responses(responses: list[dict[str, object]]) -> None:
    by_id = {
        response["id"]: response
        for response in responses
        if isinstance(response.get("id"), int)
    }
    initialize = require_result(by_id, 1)
    instructions = initialize.get("instructions")
    if not isinstance(instructions, str) or not instructions.startswith("安全边界"):
        raise RuntimeError("MCP initialize 未在开头声明远端内容安全边界")

    tools_result = require_result(by_id, 2)
    tools = tools_result.get("tools")
    if not isinstance(tools, list):
        raise RuntimeError("MCP tools/list 未返回工具数组")
    expected_tools = set(declared_mcp_tools())
    actual_tools = {tool.get("name") for tool in tools if isinstance(tool, dict)}
    if actual_tools != expected_tools:
        raise RuntimeError(
            f"MCP 工具集合不一致：缺少 {expected_tools - actual_tools}，多出 {actual_tools - expected_tools}"
        )
    validate_tool_annotations(tools)
    validate_session_tool_schemas(tools)
    for request_id, field in ((3, "sessionId"), (5, "sessionKey"), (6, "sessionId")):
        result = require_result(by_id, request_id)
        content = result.get("content")
        if (
            result.get("isError") is not True
            or not isinstance(content, list)
            or not any(
                isinstance(item, dict) and field in str(item.get("text", ""))
                for item in content
            )
            or tool_failure_code(result) is not None
        ):
            raise RuntimeError(f"MCP 缺少 {field} 时必须在参数边界失败，不能进入默认 Bridge")
    scoped_result = require_result(by_id, 4)
    structured = scoped_result.get("structuredContent")
    code = tool_failure_code(scoped_result)
    if (
        scoped_result.get("isError") is not True
        or code is None
        or not code.startswith("bridge.")
        or not isinstance(structured, dict)
        or not isinstance(structured.get("retryable"), bool)
    ):
        raise RuntimeError("显式 sessionId 的隔离调用必须保留 Bridge 错误，不能伪装成功或参数错误")


def validate_tool_annotations(tools: list[object]) -> None:
    for tool in tools:
        if not isinstance(tool, dict):
            raise RuntimeError("MCP 工具定义必须是对象")
        name = tool.get("name")
        annotations = tool.get("annotations")
        if not isinstance(name, str) or not isinstance(annotations, dict):
            raise RuntimeError("MCP 工具缺少名称或风险提示")
        hints = (
            annotations.get("readOnlyHint"),
            annotations.get("destructiveHint"),
            annotations.get("idempotentHint"),
            annotations.get("openWorldHint"),
        )
        expected = EXPECTED_TOOL_ANNOTATIONS.get(name)
        if expected is None:
            raise RuntimeError(f"MCP 工具 {name} 缺少发行风险标注")
        if hints != expected:
            raise RuntimeError(f"MCP 工具 {name} 的风险提示与真实语义不一致")


def parse_json_lines(output: str) -> list[dict[str, object]]:
    responses: list[dict[str, object]] = []
    for line in output.splitlines():
        if not line.strip():
            continue
        payload = json.loads(line)
        if not isinstance(payload, dict):
            raise RuntimeError("MCP STDIO 输出包含非对象 JSON")
        responses.append(payload)
    return responses


def require_result(
    responses: dict[int, dict[str, object]], request_id: int
) -> dict[str, object]:
    response = responses.get(request_id)
    if response is None:
        raise RuntimeError(f"MCP 缺少请求 {request_id} 的响应")
    result = response.get("result")
    if not isinstance(result, dict):
        raise RuntimeError(f"MCP 请求 {request_id} 未返回对象结果：{response}")
    return result


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"MCP 发版门禁失败：{error}", file=sys.stderr)
        raise SystemExit(1) from error
