from __future__ import annotations

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import plugin


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


if __name__ == "__main__":
    unittest.main()
