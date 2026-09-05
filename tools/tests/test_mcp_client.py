import unittest
from pathlib import Path
from unittest.mock import patch

from tools.mcp_client import (
    AGENT_ROOM_TOOLS, SESSION_SCOPED_TOOLS, McpClientFailure, McpStdioClient,
    parse_json_object, tool_failure_code, validate_session_tool_schemas,
)

SESSION_A = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a60"
SESSION_B = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a61"
SESSION_KEY = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a62"


def test_client() -> McpStdioClient:
    return McpStdioClient(
        command=("unused",), working_directory=Path.cwd(), environment={},
        stderr_path=Path("unused.log"), sanitize_line=lambda line: line,
    )


def session_tool_definitions() -> list[dict[str, object]]:
    definitions = []
    for name in AGENT_ROOM_TOOLS:
        key = "sessionKey" if name == "agent_room_open_session" else "sessionId"
        properties = {key: {"type": "string", "minLength": 36, "maxLength": 36}}
        required = [key]
        if name == "agent_room_open_session":
            properties["displayName"] = {"type": "string", "minLength": 1, "maxLength": 128}
            required.append("displayName")
        definitions.append({"name": name, "inputSchema": {
            "type": "object", "properties": properties,
            "required": required, "additionalProperties": False,
        }})
    return definitions


class McpSessionTests(unittest.TestCase):
    def test_多个绑定共享传输时每个既有工具保留各自会话(self) -> None:
        client = test_client()
        first, second = client.bind_session(SESSION_A), client.bind_session(SESSION_B)
        arguments = {"roomId": "!test:example.invalid"}
        with patch.object(client, "request", return_value={}) as request:
            for name in SESSION_SCOPED_TOOLS:
                first.call_tool_result(name, arguments)
                second.call_tool_result(name, arguments)
        self.assertEqual(len(request.call_args_list), 18)
        for index, call in enumerate(request.call_args_list):
            self.assertEqual(call.args[0], "tools/call")
            self.assertEqual(call.args[1]["arguments"], {
                **arguments, "sessionId": SESSION_A if index % 2 == 0 else SESSION_B,
            })
        self.assertNotIn("sessionId", arguments)

    def test_打开保存桥分配的句柄而非幂等键(self) -> None:
        client = test_client()
        with patch.object(client, "call_tool", return_value={
            "type": "host_session", "session": {"sessionId": SESSION_A, "state": "starting"},
        }) as call:
            session = client.open_session(session_key=SESSION_KEY, display_name="任务 A")
        self.assertEqual(session.session_id, SESSION_A)
        call.assert_called_once_with("agent_room_open_session", {
            "sessionKey": SESSION_KEY, "displayName": "任务 A",
        })

    def test_绑定拒绝非法句柄及参数覆盖(self) -> None:
        client = test_client()
        for value in ("", SESSION_A.upper(), "550e8400-e29b-41d4-a716-446655440000"):
            with self.subTest(value=value), self.assertRaises(McpClientFailure):
                client.bind_session(value)
        session = client.bind_session(SESSION_A)
        with patch.object(client, "request") as request:
            with self.assertRaises(McpClientFailure):
                session.call_tool_result("agent_room_get_self", {"sessionId": SESSION_B})
            with self.assertRaises(McpClientFailure):
                session.call_tool_result("agent_room_open_session", {})
        request.assert_not_called()

    def test_关闭必须确认同一句柄且可幂等重试(self) -> None:
        client = test_client()
        session = client.bind_session(SESSION_A)
        with patch.object(client, "call_tool", return_value={
            "type": "host_session", "session": {"sessionId": SESSION_A, "state": "closed"},
        }) as call:
            session.close()
            session.close()
        self.assertEqual(call.call_count, 2)
        call.assert_called_with("agent_room_close_session", {"sessionId": SESSION_A})
        with patch.object(client, "call_tool", return_value={
            "type": "host_session", "session": {"sessionId": SESSION_B, "state": "closed"},
        }), self.assertRaises(McpClientFailure):
            session.close()

    def test_失败会话保留嵌套稳定错误码而不泄露诊断(self) -> None:
        result = {"isError": True, "structuredContent": {
            "type": "host_session", "session": {
                "state": "failed", "sessionId": SESSION_A,
                "errorCode": "bridge.host_session.registration_failed", "details": "secret text",
            },
        }}
        client = test_client()
        with patch.object(client, "call_tool_result", return_value=result):
            with self.assertRaisesRegex(McpClientFailure, "bridge.host_session.registration_failed") as failure:
                client.open_session(session_key=SESSION_KEY, display_name="任务 A")
        self.assertNotIn("secret text", str(failure.exception))
        result["structuredContent"]["session"]["errorCode"] = "secret\ninvalid"
        self.assertIsNone(tool_failure_code(result))

    def test_打开拒绝旧身份响应及无效显示名(self) -> None:
        client = test_client()
        with patch.object(client, "call_tool", return_value={"type": "self_summary"}):
            with self.assertRaises(McpClientFailure):
                client.open_session(session_key=SESSION_KEY, display_name="任务 A")
        with patch.object(client, "call_tool") as call:
            for name in ("", " a", "a ", "a\nb", "x" * 129):
                with self.subTest(name=name), self.assertRaises(McpClientFailure):
                    client.open_session(session_key=SESSION_KEY, display_name=name)
        call.assert_not_called()


class McpSessionSchemaTests(unittest.TestCase):
    def test_十一工具都有必填的会话边界(self) -> None:
        tools = session_tool_definitions()
        self.assertEqual(len(tools), 11)
        validate_session_tool_schemas(tools)

    def test_任何既有工具把会话改为可选都失败(self) -> None:
        for name in SESSION_SCOPED_TOOLS:
            with self.subTest(name=name):
                tools = session_tool_definitions()
                next(tool for tool in tools if tool["name"] == name)["inputSchema"]["required"] = []
                with self.assertRaisesRegex(McpClientFailure, "sessionId"):
                    validate_session_tool_schemas(tools)

    def test_拒绝漏工具重复工具宽松参数及旧对象形状(self) -> None:
        for mutation in ("missing", "duplicate", "loose", "length", "display"):
            with self.subTest(mutation=mutation):
                tools = session_tool_definitions()
                if mutation == "missing":
                    tools.pop()
                elif mutation == "duplicate":
                    tools.append(tools[-1])
                elif mutation == "loose":
                    tools[2]["inputSchema"]["additionalProperties"] = True
                elif mutation == "length":
                    tools[2]["inputSchema"]["properties"]["sessionId"]["minLength"] = 0
                else:
                    tools[0]["inputSchema"]["required"].remove("displayName")
                with self.assertRaises(McpClientFailure):
                    validate_session_tool_schemas(tools)


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
