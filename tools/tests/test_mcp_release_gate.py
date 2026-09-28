from __future__ import annotations

from pathlib import Path
import json
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import mcp_release_gate as gate
from tools.mcp_client import AGENT_ROOM_TOOLS
from tools.tests.test_mcp_client import session_tool_definitions


class McpReleaseGateTests(unittest.TestCase):
    def test_rust工具声明是发行工具集合的唯一来源(self) -> None:
        declared = gate.declared_mcp_tools()

        self.assertEqual(tuple(dict.fromkeys(declared)), declared)
        self.assertEqual(set(declared), set(gate.EXPECTED_TOOL_ANNOTATIONS))
        self.assertEqual(set(declared), set(AGENT_ROOM_TOOLS))
        self.assertIn("agent_room_list_handoffs", declared)
        gate.validate_source()

    def test新增rust工具但未更新风险标注时廉价门禁立即失败(self) -> None:
        original = gate.MCP_SERVER_SOURCE.read_text(encoding="utf-8")
        injected = original + '\n#[tool(name = "agent_room_unregistered_test_tool")]\n'
        with tempfile.TemporaryDirectory(prefix="agent-room-mcp-gate-") as temporary:
            source = Path(temporary) / "server.rs"
            source.write_text(injected, encoding="utf-8")
            with patch.object(gate, "MCP_SERVER_SOURCE", source):
                with self.assertRaisesRegex(RuntimeError, "Rust 工具声明与发行风险标注不一致"):
                    gate.validate_source()

    def test_没写稳定名称的工具也会被拦下(self) -> None:
        original = gate.MCP_SERVER_SOURCE.read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory(prefix="agent-room-mcp-gate-") as temporary:
            source = Path(temporary) / "server.rs"
            source.write_text(original + "\n#[tool(description = \"无名\")]\n", encoding="utf-8")
            with patch.object(gate, "MCP_SERVER_SOURCE", source):
                with self.assertRaisesRegex(RuntimeError, "显式声明稳定名称"):
                    gate.declared_mcp_tools()

    def test_会话生命周期与安全验证工具不声称只读(self) -> None:
        for name in ("agent_room_open_session", "agent_room_close_session"):
            self.assertEqual(gate.EXPECTED_TOOL_ANNOTATIONS[name], (False, False, True, True))
        self.assertEqual(
            gate.EXPECTED_TOOL_ANNOTATIONS["agent_room_matrix_security"], (False, False, False, True)
        )

    def test_发版冒烟区分缺参错误和隔离_bridge_错误(self) -> None:
        observed = {}

        def run(command, **kwargs):
            observed.update(kwargs)
            self.assertTrue(Path(kwargs["env"]["AGENT_ROOM_BRIDGE_DATA_DIR"]).is_dir())
            # 不指定工作目录时就在一次性数据目录里跑，不碰仓库或用户目录。
            self.assertEqual(kwargs["cwd"], kwargs["env"]["AGENT_ROOM_BRIDGE_DATA_DIR"])
            return subprocess.CompletedProcess(command, 0, stdout=smoke_output(), stderr="")

        with patch.object(gate.subprocess, "run", side_effect=run), patch.object(
            gate, "declared_mcp_tools", return_value=tuple(gate.EXPECTED_TOOL_ANNOTATIONS)
        ):
            gate.smoke_test_mcp(Path("unused-binary"))
        requests = [json.loads(line) for line in observed["input"].splitlines()]
        calls = [request["params"] for request in requests if request["method"] == "tools/call"]
        self.assertEqual(calls[0]["arguments"], {})
        self.assertIn("sessionId", calls[1]["arguments"])
        self.assertEqual(calls[2], {"name": "agent_room_open_session", "arguments": {}})
        self.assertTrue(
            observed["env"]["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"].startswith("dev.agent-room.smoke.")
        )

    def test_旧二进制或只会返回桥错误不能通过会话冒烟(self) -> None:
        for request_id, bad in (
            (3, {"isError": True, "structuredContent": {"code": "bridge.unavailable"}, "content": []}),
            (4, {"isError": False, "structuredContent": {"type": "self_summary"}}),
            (5, {"isError": False, "content": []}),
            (6, {"isError": True, "content": [{"type": "text", "text": "other error"}]}),
        ):
            with self.subTest(request_id=request_id), patch.object(
                gate.subprocess, "run",
                return_value=subprocess.CompletedProcess([], 0, stdout=smoke_output({request_id: bad}), stderr=""),
            ), patch.object(gate, "declared_mcp_tools", return_value=tuple(gate.EXPECTED_TOOL_ANNOTATIONS)):
                with self.assertRaises(RuntimeError):
                    gate.smoke_test_mcp(Path("unused-binary"))

    def test_风险提示与标注不一致时冒烟失败(self) -> None:
        tools = session_tool_definitions()
        annotate(tools)
        tools[0]["annotations"]["readOnlyHint"] = not tools[0]["annotations"]["readOnlyHint"]
        with self.assertRaisesRegex(RuntimeError, "风险提示与真实语义不一致"):
            gate.validate_tool_annotations(tools)

    def test_smoke_要用已有的发行二进制(self) -> None:
        with tempfile.TemporaryDirectory(prefix="agent-room-mcp-gate-") as temporary:
            missing = Path(temporary) / "agent-room-mcp"
            with self.assertRaisesRegex(RuntimeError, "MCP 二进制不存在"):
                gate.resolve_binary(missing)
            missing.write_bytes(b"binary")
            self.assertEqual(gate.resolve_binary(missing), missing.resolve())

    def test_validate_不启动任何进程(self) -> None:
        with patch.object(gate.subprocess, "run", side_effect=AssertionError("不应启动进程")):
            gate.main(["validate"])


def annotate(tools: list[dict[str, object]]) -> None:
    for tool in tools:
        tool["annotations"] = dict(zip(
            ("readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"),
            gate.EXPECTED_TOOL_ANNOTATIONS[tool["name"]], strict=True,
        ))


def smoke_output(overrides=None) -> str:
    tools = session_tool_definitions()
    annotate(tools)
    results = {
        1: {"instructions": "安全边界：测试"}, 2: {"tools": tools},
        4: {"isError": True, "structuredContent": {"code": "bridge.ipc.unavailable", "retryable": True}, "content": []},
    }
    for request_id, field in ((3, "sessionId"), (5, "sessionKey"), (6, "sessionId")):
        results[request_id] = {"isError": True, "content": [{"type": "text", "text": f"missing field {field}"}]}
    results.update(overrides or {})
    return "\n".join(json.dumps({"jsonrpc": "2.0", "id": key, "result": value}) for key, value in results.items())


if __name__ == "__main__":
    unittest.main()
