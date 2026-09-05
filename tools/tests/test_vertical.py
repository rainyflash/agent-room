from contextlib import ExitStack, nullcontext
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from tools import vertical
from tools.local_runtime import ControlPlaneNetworkScope
from tools.mcp_client import McpClientFailure
from tools.tests.test_mcp_client import (
    SESSION_A, SESSION_B, SESSION_KEY, session_tool_definitions, test_client,
)
from tools.vertical import (
    BridgeRuntimeObservation,
    IsolatedBridgeState,
    IsolatedServiceInterruption,
    LogRedactor,
    VerticalFailure,
    compose_command,
    http_status_is_ready,
    linux_secret_clear_succeeded,
    new_uuid_v7,
    read_string_object,
    require_uuid_v7,
    verify_sanitized_logs,
    vertical_control_plane_environment,
    web_preview_command,
    windows_credential_target,
)


class RunningProcess:
    def ensure_running(self) -> None:
        return


DEFAULT_AGENT = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a70"
TASK_AGENT = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a71"
TASK_INSTANCE = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a72"
SENDER_AGENT = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a73"
SENDER_INSTANCE = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a74"
ROOM_ID = "!vertical:example.invalid"


def identity_result(
    agent_id: str = TASK_AGENT, instance_id: str = TASK_INSTANCE
) -> dict[str, object]:
    return {
        "structuredContent": {
            "type": "self_summary",
            "summary": {
                "agent": {
                    "agentId": agent_id,
                    "matrixUserId": f"@agent_{agent_id}:example.invalid",
                },
                "instanceId": instance_id,
                "matrixDeviceId": f"AR_{instance_id}",
                "connectionState": "ready",
                "roomId": ROOM_ID,
            },
        },
        "isError": False,
    }


def lifecycle_result(session_id: str, state: str) -> dict[str, object]:
    return {"structuredContent": {
        "type": "host_session",
        "session": {"sessionId": session_id, "state": state},
    }}


def bridge_fixture() -> vertical.AuthorizedBridgeRuntime:
    return vertical.AuthorizedBridgeRuntime(
        process=MagicMock(spec=vertical.ManagedProcess),
        environment={"AGENT_ROOM_AGENT_ID": DEFAULT_AGENT},
        observation=BridgeRuntimeObservation(),
        device_code="ABCD-EFGH",
        session_key=SESSION_KEY,
        display_name="Vertical bridge-target",
    )


class VerticalSessionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.enterContext(patch.object(vertical, "runtime_binary", return_value=Path("unused-binary")))

    def test_初始化仅重试原会话且重开保留任务键和真实身份(self) -> None:
        runtime = bridge_fixture()
        transport = test_client()
        retryable = {"isError": True, "structuredContent": {
            "code": "bridge.host_session.starting", "retryable": True,
        }}
        responses = [
            lifecycle_result(SESSION_A, "starting"), retryable, identity_result(),
            lifecycle_result(SESSION_A, "closed"),
            lifecycle_result(SESSION_B, "starting"), identity_result(),
        ]
        with (
            patch.object(transport, "request", side_effect=responses) as request,
            patch.object(vertical, "bridge_mcp_client", return_value=nullcontext(transport)),
            patch.object(vertical.time, "sleep"),
        ):
            vertical.open_bridge_session(runtime, LogRedactor({}))
            first = dict(vertical.require_bridge_session(runtime))
            vertical.close_bridge_session(runtime, LogRedactor({}))
            vertical.open_bridge_session(runtime, LogRedactor({}))
        self.assertEqual(first["sessionId"], SESSION_A)
        self.assertEqual(runtime.session, {**first, "sessionId": SESSION_B})
        self.assertEqual(first["agentId"], TASK_AGENT)
        calls = [call.args[1] for call in request.call_args_list]
        self.assertEqual(calls[0], calls[4])
        self.assertEqual(calls[0]["arguments"], {
            "sessionKey": SESSION_KEY, "displayName": runtime.display_name,
        })
        self.assertEqual(
            [call["arguments"]["sessionId"] for call in calls if "sessionId" in call["arguments"]],
            [SESSION_A, SESSION_A, SESSION_A, SESSION_B],
        )

    def test_旧默认身份与恢复后的身份漂移不能冒充新会话(self) -> None:
        cases = [
            (DEFAULT_AGENT, TASK_INSTANCE, False),
            (SENDER_AGENT, TASK_INSTANCE, True),
            (TASK_AGENT, SENDER_INSTANCE, True),
        ]
        for agent_id, instance_id, restoring in cases:
            with self.subTest(agent_id=agent_id, instance_id=instance_id):
                runtime = bridge_fixture()
                if restoring:
                    runtime.session = {
                        **vertical.verify_mcp_identity_response(identity_result()["structuredContent"]),
                        "sessionId": SESSION_A,
                    }
                previous = runtime.session
                transport = test_client()
                with (
                    patch.object(transport, "request", side_effect=[
                        lifecycle_result(SESSION_B, "ready"),
                        identity_result(agent_id, instance_id),
                    ]),
                    patch.object(vertical, "bridge_mcp_client", return_value=nullcontext(transport)),
                    self.assertRaises(VerticalFailure),
                ):
                    vertical.open_bridge_session(runtime, LogRedactor({}))
                self.assertIs(runtime.session, previous)

    def test_初始化终态立即失败并保留错误码(self) -> None:
        for retryable in (False, None):
            with self.subTest(retryable=retryable):
                transport = test_client()
                result = {"isError": True, "structuredContent": {
                    "code": "bridge.host_session.closed", "retryable": retryable,
                    "details": "private diagnostic",
                }}
                with (
                    patch.object(transport, "request", return_value=result) as request,
                    patch.object(vertical.time, "sleep") as sleep,
                    self.assertRaisesRegex(VerticalFailure, "bridge.host_session.closed") as failure,
                ):
                    vertical.wait_for_session_identity(
                        transport.bind_session(SESSION_A), timeout_seconds=10,
                    )
                request.assert_called_once()
                sleep.assert_not_called()
                self.assertNotIn("private diagnostic", str(failure.exception))

    def test_初始化超时保留最后错误且不换会话(self) -> None:
        transport = test_client()
        with (
            patch.object(transport, "request", return_value={"isError": True, "structuredContent": {
                "code": "bridge.host_session.starting", "retryable": True,
            }}) as request,
            patch.object(vertical.time, "monotonic", side_effect=[0, 0, 2]),
            patch.object(vertical.time, "sleep"),
            self.assertRaisesRegex(VerticalFailure, "bridge.host_session.starting"),
        ):
            vertical.wait_for_session_identity(transport.bind_session(SESSION_A), timeout_seconds=1)
        request.assert_called_once_with("tools/call", {
            "name": "agent_room_get_self", "arguments": {"sessionId": SESSION_A},
        })

    def test_断网验收使用原会话且拒绝把永久错误当成可恢复(self) -> None:
        for retryable in (True, False):
            with self.subTest(retryable=retryable):
                transport = test_client()
                with (
                    patch.object(transport, "request", return_value={"isError": True, "structuredContent": {
                        "code": "bridge.agent_runtime_unavailable", "retryable": retryable,
                    }}) as request,
                    patch.object(vertical, "McpStdioClient", return_value=nullcontext(transport)),
                    nullcontext() if retryable else self.assertRaisesRegex(
                        VerticalFailure, "bridge.agent_runtime_unavailable"
                    ),
                ):
                    vertical.wait_for_mcp_runtime_unavailable(
                        bridge_environment={}, bridge_process=RunningProcess(),
                        redactor=LogRedactor({}), session_id=SESSION_A, timeout_seconds=1,
                    )
                request.assert_called_once_with("tools/call", {
                    "name": "agent_room_get_self", "arguments": {"sessionId": SESSION_A},
                })

    def test_桌面交接通过环境传递真实会话而非默认人物(self) -> None:
        parameters = {
            "bridge_environment": {"AGENT_ROOM_AGENT_ID": DEFAULT_AGENT},
            "principal_id": SESSION_KEY, "room_id": ROOM_ID,
            "source_content_id": TASK_AGENT, "target_agent_id": TASK_AGENT,
            "target_instance_id": TASK_INSTANCE,
        }
        with (
            patch.object(vertical, "run_checked") as run,
            patch.object(vertical, "executable", return_value="cargo"),
        ):
            handoff_id = vertical.approve_real_handoff(session_id=SESSION_A, **parameters)
            environment = run.call_args.kwargs["environment"]
            self.assertEqual(environment["AGENT_ROOM_TEST_SESSION_ID"], SESSION_A)
            self.assertEqual(environment["AGENT_ROOM_TEST_TARGET_AGENT_ID"], TASK_AGENT)
            self.assertEqual(environment["AGENT_ROOM_TEST_HANDOFF_ID"], handoff_id)
            self.assertNotIn("AGENT_ROOM_TEST_SESSION_ID", parameters["bridge_environment"])
            run.reset_mock()
            with self.assertRaises(McpClientFailure):
                vertical.approve_real_handoff(session_id="", **parameters)
            run.assert_not_called()

    def test_纵向消息与交接路由使用两个真实任务的绑定(self) -> None:
        target, sender = bridge_fixture(), bridge_fixture()
        target.session = {
            **vertical.verify_mcp_identity_response(identity_result()["structuredContent"]),
            "sessionId": SESSION_A,
        }
        sender.session = {
            **vertical.verify_mcp_identity_response(
                identity_result(SENDER_AGENT, SENDER_INSTANCE)["structuredContent"]
            ),
            "sessionId": SESSION_B,
        }
        transport = test_client()

        def respond(method, parameters):
            if method == "tools/list":
                return {"tools": session_tool_definitions()}
            self.assertEqual(parameters["name"], "agent_room_get_self")
            if parameters["arguments"]["sessionId"] == SESSION_A:
                return identity_result()
            self.assertEqual(parameters["arguments"]["sessionId"], SESSION_B)
            return identity_result(SENDER_AGENT, SENDER_INSTANCE)

        helper_results = {
            "active_room_for_agent": {"matrixRoomId": ROOM_ID, "roomInstanceId": SESSION_KEY},
            "verify_mcp_status_publication": None,
            "wait_for_mcp_presence": None,
            "send_mcp_vertical_message": {"eventId": "$message", "body": "test"},
            "wait_for_mcp_preview": {"messageId": TASK_AGENT, "content": {"contentId": TASK_AGENT}},
            "verify_mcp_opened_content": None,
            "approve_real_handoff": SESSION_KEY,
            "wait_for_mcp_handoff_consumption": None,
            "send_mcp_vertical_reply": {"eventId": "$reply", "body": "reply"},
        }
        with ExitStack() as stack:
            stack.enter_context(patch.object(transport, "request", side_effect=respond))
            stack.enter_context(patch.object(vertical, "McpStdioClient", return_value=nullcontext(transport)))
            helpers = {
                name: stack.enter_context(patch.object(vertical, name, return_value=result))
                for name, result in helper_results.items()
            }
            result = vertical.verify_mcp_workflow(
                target_bridge=target, sender_bridge=sender,
                principal_id=SESSION_KEY, redactor=LogRedactor({}),
            )
        helpers["active_room_for_agent"].assert_called_once_with(TASK_AGENT)
        self.assertEqual(result["agentId"], TASK_AGENT)
        self.assertEqual(result["senderAgentId"], SENDER_AGENT)
        self.assertEqual(helpers["send_mcp_vertical_message"].call_args.args[0].session_id, SESSION_B)
        for name in (
            "verify_mcp_status_publication", "wait_for_mcp_presence",
            "wait_for_mcp_preview", "verify_mcp_opened_content",
            "wait_for_mcp_handoff_consumption", "send_mcp_vertical_reply",
        ):
            for call in helpers[name].call_args_list:
                self.assertEqual(call.args[0].session_id, SESSION_A)
        approval = helpers["approve_real_handoff"].call_args.kwargs
        self.assertEqual(approval["session_id"], SESSION_B)
        self.assertEqual(approval["target_agent_id"], TASK_AGENT)
        self.assertEqual(approval["target_instance_id"], TASK_INSTANCE)


class LogRedactorTests(unittest.TestCase):
    def test_日志在落盘前移除已知凭据和设备码(self) -> None:
        redactor = LogRedactor(
            {
                "KEYCLOAK_CLIENT_SECRET": "local-secret-value",
                "SAFE_PORT": "8090",
            }
        )

        text = redactor.redact(
            "local-secret-value user_code=ABCD-EFGH\n设备验证码：ABCD-EFGH\n"
        )

        self.assertNotIn("local-secret-value", text)
        self.assertNotIn("ABCD-EFGH", text)
        self.assertIn("[已脱敏]", text)

    def test_设备码只进入内存观察器而不会进入日志(self) -> None:
        observation = BridgeRuntimeObservation()
        redactor = LogRedactor({})
        line = "设备验证码：ABCD-EFGH\n"

        observation.observe(line)

        code = observation.wait_for_device_code(
            RunningProcess(), timeout_seconds=0.1
        )
        self.assertEqual(code, "ABCD-EFGH")
        self.assertNotIn(code, redactor.redact(line))

    def test_反向日志扫描接受脱敏内容(self) -> None:
        redactor = LogRedactor({"CLIENT_SECRET": "local-secret-value"})
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "bridge.log"
            log.write_text(
                "设备验证码：[已脱敏]\ncallback?user_code=[已脱敏]\n",
                encoding="utf-8",
            )

            scanned = verify_sanitized_logs(
                (log,), redactor, additional_secrets=("ABCD-EFGH",)
            )

        self.assertEqual(scanned, ("bridge.log",))

    def test_反向日志扫描拒绝任何敏感值残留(self) -> None:
        cases = {
            "known-secret": "local-secret-value",
            "jwt": "eyJaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbb.cccccccccccccccc",
            "device-query": "callback?user_code=ABCD-EFGH",
            "device-line": "设备验证码：ABCD-EFGH",
        }
        for name, content in cases.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                log = Path(directory) / "bridge.log"
                log.write_text(f"{content}\n", encoding="utf-8")
                with self.assertRaises(VerticalFailure):
                    verify_sanitized_logs(
                        (log,),
                        LogRedactor({"CLIENT_SECRET": "local-secret-value"}),
                        additional_secrets=("ABCD-EFGH",),
                    )


class BridgeObservationTests(unittest.TestCase):
    def test_区分首次上线与恢复后的新在线代次(self) -> None:
        observation = BridgeRuntimeObservation()
        process = RunningProcess()
        observation.observe("Agent 已进入公共大厅并开始同步。\n")
        initial = observation.wait_for_agent_online(process, timeout_seconds=0.1)

        observation.observe("Agent 已进入公共大厅并开始同步。\n")
        recovered = observation.wait_for_agent_online(
            process,
            after_generation=initial,
            timeout_seconds=0.1,
        )

        self.assertEqual(initial, 1)
        self.assertEqual(recovered, 2)


class ResultValidationTests(unittest.TestCase):
    def test_生成标准_uuidv7(self) -> None:
        value = new_uuid_v7()

        require_uuid_v7(value, "测试标识")

    def test_只接受字符串对象结果(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text('{"agentId": 7}\n', encoding="utf-8")

            with self.assertRaises(VerticalFailure):
                read_string_object(result)

    def test_uuidv7_校验拒绝普通_uuid(self) -> None:
        require_uuid_v7("019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a60", "测试标识")
        with self.assertRaises(VerticalFailure):
            require_uuid_v7("550e8400-e29b-41d4-a716-446655440000", "测试标识")


class HttpReadinessTests(unittest.TestCase):
    def test_只有成功状态才表示服务就绪(self) -> None:
        self.assertTrue(http_status_is_ready(200))
        self.assertTrue(http_status_is_ready(204))
        self.assertFalse(http_status_is_ready(302))
        self.assertFalse(http_status_is_ready(404))
        self.assertFalse(http_status_is_ready(503))

    def test_发布预览监听容器可达地址(self) -> None:
        with patch("tools.vertical.executable", return_value="node"):
            command = web_preview_command()

        host_index = command.index("--host")
        self.assertEqual(command[host_index + 1], "0.0.0.0")

    def test_纵向控制平面监听容器可达地址(self) -> None:
        with patch(
            "tools.vertical.control_plane_runtime_environment",
            return_value={"AGENT_ROOM_BIND_ADDRESS": "0.0.0.0:8090"},
        ) as build_environment:
            environment = vertical_control_plane_environment({})

        build_environment.assert_called_once_with(
            {},
            enable_telemetry=True,
            network_scope=ControlPlaneNetworkScope.DOCKER_GATEWAY,
        )
        self.assertEqual(environment["AGENT_ROOM_BIND_ADDRESS"], "0.0.0.0:8090")


class ComposeBoundaryTests(unittest.TestCase):
    def test_子会话凭据按测试安装命名空间分别寻址(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data_roots = (root / "bridge-sender", root / "bridge-target")
            for data_root in data_roots:
                (data_root / "host-agents" / TASK_AGENT).mkdir(parents=True)
            with (
                patch.object(vertical, "VERTICAL_ROOT", root),
                patch.object(vertical, "BRIDGE_DATA_ROOTS", data_roots),
            ):
                services = vertical.vertical_secure_storage_services()
        self.assertEqual(services[:2], vertical.SECURE_STORAGE_SERVICES)
        self.assertEqual(len(set(services)), 4)
        for service in services[2:]:
            self.assertRegex(service, r"^dev\.agent-room\.host\.[A-Za-z0-9_-]{43}\.v1$")
            self.assertNotIn(TASK_AGENT, service)

    def test_清理凭据前保留新旧子会话目录供寻址(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data_roots = (root / "bridge-sender", root / "bridge-target")
            old_child = data_roots[0] / "host-agents" / TASK_AGENT
            new_child = data_roots[1] / "host-agents" / SENDER_AGENT
            old_child.mkdir(parents=True)
            observed = []

            def clear() -> None:
                observed.append((old_child.exists(), new_child.exists()))

            with (
                patch.object(vertical, "VERTICAL_ROOT", root),
                patch.object(vertical, "BRIDGE_DATA_ROOTS", data_roots),
                patch.object(vertical, "clear_vertical_secure_storage", side_effect=clear),
            ):
                with IsolatedBridgeState():
                    self.assertFalse(old_child.exists())
                    new_child.mkdir(parents=True)
            self.assertEqual(observed, [(True, False), (False, True)])
            self.assertTrue(all(not data_root.exists() for data_root in data_roots))

    def test_凭据清理失败时保留目录并传播错误(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data_root = root / "bridge-sender"
            child = data_root / "host-agents" / TASK_AGENT
            child.mkdir(parents=True)
            with (
                patch.object(vertical, "VERTICAL_ROOT", root),
                patch.object(vertical, "BRIDGE_DATA_ROOTS", (data_root,)),
                patch.object(vertical, "clear_vertical_secure_storage", side_effect=VerticalFailure("test failure")),
                self.assertRaisesRegex(VerticalFailure, "test failure"),
            ):
                IsolatedBridgeState().__exit__(None, None, None)
            self.assertTrue(child.is_dir())

    def test_子会话清理拒绝测试根以外目录与非法人物标识(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data_root = root / "bridge-sender"
            children = data_root / "host-agents"
            invalid = children / "550e8400-e29b-41d4-a716-446655440000"
            invalid.mkdir(parents=True)
            with (
                patch.object(vertical, "VERTICAL_ROOT", root),
                patch.object(vertical, "BRIDGE_DATA_ROOTS", (data_root, root / "bridge-target")),
                self.assertRaisesRegex(VerticalFailure, "Agent ID 无效"),
            ):
                vertical.vertical_secure_storage_services()
            with (
                patch.object(vertical, "VERTICAL_ROOT", root),
                patch.object(vertical, "BRIDGE_DATA_ROOTS", (root.parent, root / "bridge-target")),
                self.assertRaisesRegex(VerticalFailure, "测试目录之外"),
            ):
                vertical.vertical_secure_storage_services()

    def test_拒绝操作未登记的_compose_项目(self) -> None:
        with self.assertRaises(VerticalFailure):
            compose_command("agent-room-user-data")

    def test_windows_凭据目标与_rust_后端映射一致(self) -> None:
        self.assertEqual(
            windows_credential_target("dev.agent-room.bridge.vertical-24", "session"),
            "session.dev.agent-room.bridge.vertical-24",
        )

    def test_linux_凭据清理把无匹配视为幂等成功(self) -> None:
        self.assertTrue(
            linux_secret_clear_succeeded(
                subprocess.CompletedProcess([], 1, stdout=b"", stderr=b"")
            )
        )

    def test_linux_凭据清理不会吞掉_secret_service_错误(self) -> None:
        self.assertFalse(
            linux_secret_clear_succeeded(
                subprocess.CompletedProcess(
                    [], 1, stdout=b"", stderr=b"D-Bus service unavailable"
                )
            )
        )

    @unittest.skipUnless(os.name == "posix", "仅 POSIX 平台支持目录权限位")
    def test_隔离_bridge_目录使用私有权限并在退出时清理(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data_roots = (root / "bridge-sender", root / "bridge-target")
            with (
                patch("tools.vertical.VERTICAL_ROOT", root),
                patch("tools.vertical.BRIDGE_DATA_ROOTS", data_roots),
                patch("tools.vertical.clear_vertical_secure_storage"),
            ):
                with IsolatedBridgeState():
                    for data_root in data_roots:
                        permissions = stat.S_IMODE(data_root.stat().st_mode)
                        self.assertEqual(permissions, 0o700)

                for data_root in data_roots:
                    self.assertFalse(data_root.exists())

    def test_拒绝中断未登记的基础设施服务(self) -> None:
        with self.assertRaises(VerticalFailure):
            IsolatedServiceInterruption("postgres")


if __name__ == "__main__":
    unittest.main()
