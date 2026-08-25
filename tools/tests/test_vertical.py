from pathlib import Path
import tempfile
import unittest

from tools.vertical import (
    BridgeRuntimeObservation,
    LogRedactor,
    VerticalFailure,
    compose_command,
    http_status_is_ready,
    new_uuid_v7,
    read_string_object,
    require_uuid_v7,
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


class ComposeBoundaryTests(unittest.TestCase):
    def test_拒绝操作未登记的_compose_项目(self) -> None:
        with self.assertRaises(VerticalFailure):
            compose_command("agent-room-user-data")

    def test_windows_凭据目标与_rust_后端映射一致(self) -> None:
        self.assertEqual(
            windows_credential_target("dev.agent-room.bridge.vertical-24", "session"),
            "session.dev.agent-room.bridge.vertical-24",
        )


if __name__ == "__main__":
    unittest.main()
