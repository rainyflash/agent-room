from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools.vertical import (
    BridgeRuntimeObservation,
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
            return_value={"AGENT_ROOM_BIND_ADDRESS": "127.0.0.1:8090"},
        ):
            environment = vertical_control_plane_environment({})

        self.assertEqual(environment["AGENT_ROOM_BIND_ADDRESS"], "0.0.0.0:8090")


class ComposeBoundaryTests(unittest.TestCase):
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

    def test_拒绝中断未登记的基础设施服务(self) -> None:
        with self.assertRaises(VerticalFailure):
            IsolatedServiceInterruption("postgres")


if __name__ == "__main__":
    unittest.main()
