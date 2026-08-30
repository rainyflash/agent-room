import unittest
from pathlib import Path
from unittest.mock import patch

from tools.mcp_client import McpClientFailure, McpStdioClient, parse_json_object


class McpResponseValidationTests(unittest.TestCase):
    def test_只接受_json_对象(self) -> None:
        self.assertEqual(parse_json_object('{"ok":true}', "响应"), {"ok": True})
        with self.assertRaises(McpClientFailure):
            parse_json_object("[]", "响应")

    def test_拒绝损坏的_json(self) -> None:
        with self.assertRaises(McpClientFailure):
            parse_json_object("{", "响应")

    def test_工具失败只暴露稳定错误码(self) -> None:
        client = McpStdioClient(
            command=("unused",),
            working_directory=Path.cwd(),
            environment={},
            stderr_path=Path("unused.log"),
            sanitize_line=lambda line: line,
        )
        result = {
            "isError": True,
            "structuredContent": {
                "code": "bridge.message_matrix_unavailable",
                "details": "禁止进入诊断异常文本",
            },
        }
        with patch.object(client, "call_tool_result", return_value=result):
            with self.assertRaisesRegex(
                McpClientFailure,
                r"错误码 bridge\.message_matrix_unavailable",
            ):
                client.call_tool("agent_room_send_message", {})

    def test_工具失败拒绝把任意文本当成错误码(self) -> None:
        client = McpStdioClient(
            command=("unused",),
            working_directory=Path.cwd(),
            environment={},
            stderr_path=Path("unused.log"),
            sanitize_line=lambda line: line,
        )
        result = {
            "isError": True,
            "structuredContent": {"code": "bad code\nsecret"},
        }
        with patch.object(client, "call_tool_result", return_value=result):
            with self.assertRaisesRegex(
                McpClientFailure,
                r"MCP 工具 agent_room_send_message 返回失败。$",
            ):
                client.call_tool("agent_room_send_message", {})


if __name__ == "__main__":
    unittest.main()
