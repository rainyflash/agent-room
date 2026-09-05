from __future__ import annotations

from pathlib import Path
import json
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import plugin
from tools.tests.test_mcp_client import session_tool_definitions


class PluginContractTests(unittest.TestCase):
    def test_rust工具声明是发行工具集合的唯一来源(self) -> None:
        declared = plugin.declared_mcp_tools()

        self.assertEqual(tuple(dict.fromkeys(declared)), declared)
        self.assertEqual(set(declared), set(plugin.EXPECTED_TOOL_ANNOTATIONS))
        self.assertIn("agent_room_list_handoffs", declared)

    def test新增rust工具但未更新审批策略时廉价门禁立即失败(self) -> None:
        original = plugin.MCP_SERVER_SOURCE.read_text(encoding="utf-8")
        injected = original + '\n#[tool(name = "agent_room_unregistered_test_tool")]\n'
        with tempfile.TemporaryDirectory(prefix="agent-room-plugin-contract-") as temporary:
            source = Path(temporary) / "server.rs"
            source.write_text(injected, encoding="utf-8")
            with patch.object(plugin, "MCP_SERVER_SOURCE", source):
                with self.assertRaisesRegex(
                    RuntimeError,
                    "Rust 工具声明与发行风险标注不一致",
                ):
                    plugin.validate_source()

    def test_会话生命周期工具没有被自动批准(self) -> None:
        plugin.validate_approval_policy(
            plugin.PLUGIN_SOURCE / "approval-policy.example.toml",
            tuple(plugin.EXPECTED_TOOL_ANNOTATIONS),
        )
        for name in ("agent_room_open_session", "agent_room_close_session"):
            self.assertNotIn(name, plugin.AUTOMATIC_TOOLS)
            self.assertEqual(plugin.EXPECTED_TOOL_ANNOTATIONS[name], (False, False, True, True))

    def test_打包冒烟区分缺参错误和隔离_bridge_错误(self) -> None:
        observed = {}

        def run(command, **kwargs):
            observed.update(kwargs)
            self.assertTrue(Path(kwargs["env"]["AGENT_ROOM_BRIDGE_DATA_DIR"]).is_dir())
            return subprocess.CompletedProcess(command, 0, stdout=smoke_output(), stderr="")

        with patch.object(plugin.subprocess, "run", side_effect=run), patch.object(
            plugin, "declared_mcp_tools", return_value=tuple(plugin.EXPECTED_TOOL_ANNOTATIONS)
        ):
            plugin.smoke_test_mcp(Path("unused-binary"), Path.cwd())
        requests = [json.loads(line) for line in observed["input"].splitlines()]
        calls = [request["params"] for request in requests if request["method"] == "tools/call"]
        self.assertEqual(calls[0]["arguments"], {})
        self.assertIn("sessionId", calls[1]["arguments"])
        self.assertEqual(calls[2], {"name": "agent_room_open_session", "arguments": {}})
        self.assertTrue(observed["env"]["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"].startswith("dev.agent-room.smoke."))

    def test_旧二进制或只会返回桥错误不能通过会话冒烟(self) -> None:
        for request_id, bad in (
            (3, {"isError": True, "structuredContent": {"code": "bridge.unavailable"}, "content": []}),
            (4, {"isError": False, "structuredContent": {"type": "self_summary"}}),
            (5, {"isError": False, "content": []}),
            (6, {"isError": True, "content": [{"type": "text", "text": "other error"}]}),
        ):
            with self.subTest(request_id=request_id), patch.object(
                plugin.subprocess, "run",
                return_value=subprocess.CompletedProcess([], 0, stdout=smoke_output({request_id: bad}), stderr=""),
            ), patch.object(plugin, "declared_mcp_tools", return_value=tuple(plugin.EXPECTED_TOOL_ANNOTATIONS)):
                with self.assertRaises(RuntimeError):
                    plugin.smoke_test_mcp(Path("unused-binary"), Path.cwd())


def smoke_output(overrides=None) -> str:
    tools = session_tool_definitions()
    for tool in tools:
        tool["annotations"] = dict(zip(
            ("readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"),
            plugin.EXPECTED_TOOL_ANNOTATIONS[tool["name"]], strict=True,
        ))
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
