import os
from pathlib import Path
import unittest
from unittest.mock import patch

from tools.database import compose_project_name
from tools.local_runtime import (
    LocalRuntimeError,
    bridge_runtime_environment,
    control_plane_runtime_environment,
)


REQUIRED_VALUES = {
    "AGENT_ROOM_DB_RUNTIME_PASSWORD": "runtime-password",
    "KEYCLOAK_CLIENT_SECRET": "oidc-secret",
    "SYNAPSE_APPSERVICE_TOKEN": "appservice-token",
    "S3_ACCESS_KEY": "access-key",
    "S3_SECRET_KEY": "storage-secret",
    "CONTENT_TICKET_SECRET": "ticket-secret",
    "CONTENT_MATRIX_AGENT_ID": "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4",
}


class ControlPlaneEnvironmentTests(unittest.TestCase):
    def test_遥测开关不会残留旧端点(self) -> None:
        with patch.dict(
            os.environ,
            {"AGENT_ROOM_OTLP_TRACES_ENDPOINT": "https://stale.invalid"},
        ):
            disabled = control_plane_runtime_environment(
                REQUIRED_VALUES, enable_telemetry=False
            )
            enabled = control_plane_runtime_environment(
                REQUIRED_VALUES, enable_telemetry=True
            )

        self.assertNotIn("AGENT_ROOM_OTLP_TRACES_ENDPOINT", disabled)
        self.assertEqual(
            enabled["AGENT_ROOM_OTLP_TRACES_ENDPOINT"],
            "http://127.0.0.1:14318/v1/traces",
        )

    def test_缺失凭据时立即失败(self) -> None:
        with self.assertRaises(LocalRuntimeError):
            control_plane_runtime_environment({}, enable_telemetry=False)


class BridgeEnvironmentTests(unittest.TestCase):
    def test_纵向运行显式隔离身份目录和安全存储(self) -> None:
        environment = bridge_runtime_environment(
            data_root=Path("C:/agent-room/vertical").resolve(),
            agent_id="01945c1e-7b5a-7c7f-8a28-2de53f56a9a4",
            public_lobby_catalog_id="01945c1e-7b5b-7c7f-8a28-2de53f56a9a5",
            secure_storage_service="dev.agent-room.bridge.vertical-24",
            base_environment={"AGENT_ROOM_LOBBY_REGION": "stale"},
        )

        self.assertEqual(environment["AGENT_ROOM_LOBBY_LANGUAGE"], "en")
        self.assertEqual(
            environment["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"],
            "dev.agent-room.bridge.vertical-24",
        )
        self.assertNotIn("AGENT_ROOM_LOBBY_REGION", environment)

    def test_agent_与大厅配置拒绝半套状态(self) -> None:
        with self.assertRaises(LocalRuntimeError):
            bridge_runtime_environment(
                data_root=Path("C:/agent-room/vertical").resolve(),
                agent_id="01945c1e-7b5a-7c7f-8a28-2de53f56a9a4",
                base_environment={},
            )

    def test_拒绝不安全的安全存储命名空间(self) -> None:
        with self.assertRaises(LocalRuntimeError):
            bridge_runtime_environment(
                data_root=Path("C:/agent-room/vertical").resolve(),
                secure_storage_service="../共享凭据",
                base_environment={},
            )


class DatabaseProjectBoundaryTests(unittest.TestCase):
    def test_只接受安全的_compose_项目名(self) -> None:
        with patch.dict(
            os.environ,
            {"AGENT_ROOM_COMPOSE_PROJECT_NAME": "agent-room-vertical-24"},
        ):
            self.assertEqual(compose_project_name(), "agent-room-vertical-24")

        with patch.dict(
            os.environ,
            {"AGENT_ROOM_COMPOSE_PROJECT_NAME": "../user-data"},
        ):
            with self.assertRaises(RuntimeError):
                compose_project_name()


if __name__ == "__main__":
    unittest.main()
