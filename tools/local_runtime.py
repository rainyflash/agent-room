"""集中生成本地进程配置，避免每个启动器复制敏感环境装配。"""

from __future__ import annotations

from collections.abc import Mapping
import os
from pathlib import Path
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"


class LocalRuntimeError(RuntimeError):
    """表示本地环境缺失或不完整。"""


def read_environment(path: Path = ENV_FILE) -> dict[str, str]:
    if not path.is_file():
        raise LocalRuntimeError("缺少 .env.local；请先准备本地基础设施。")

    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        name, separator, value = line.partition("=")
        if not separator or not name:
            raise LocalRuntimeError(".env.local 包含无效配置行。")
        values[name] = value
    return values


def required_value(values: Mapping[str, str], name: str) -> str:
    value = values.get(name)
    if value is None or not value.strip():
        raise LocalRuntimeError(f".env.local 缺少 {name}。")
    return value


def control_plane_runtime_environment(
    values: Mapping[str, str], *, enable_telemetry: bool
) -> dict[str, str]:
    environment = os.environ.copy()
    environment.update(
        {
            "AGENT_ROOM_BIND_ADDRESS": "127.0.0.1:8090",
            "AGENT_ROOM_DB_HOST": "127.0.0.1",
            "AGENT_ROOM_DB_PORT": "55432",
            "AGENT_ROOM_DB_NAME": "agent_room",
            "AGENT_ROOM_DB_USER": "agent_room_runtime",
            "AGENT_ROOM_DB_RUNTIME_PASSWORD": required_value(
                values, "AGENT_ROOM_DB_RUNTIME_PASSWORD"
            ),
            "AGENT_ROOM_DB_TLS_MODE": "disable",
            "AGENT_ROOM_MATRIX_BASE_URL": "http://127.0.0.1:18008",
            "AGENT_ROOM_MATRIX_APPSERVICE_TOKEN": required_value(
                values, "SYNAPSE_APPSERVICE_TOKEN"
            ),
            "AGENT_ROOM_OBJECT_STORE_HEALTH_URL": (
                "http://127.0.0.1:19333/cluster/status"
            ),
            "AGENT_ROOM_DEPENDENCY_TIMEOUT_MS": "2000",
            "AGENT_ROOM_CONTENT_S3_ENDPOINT": "http://127.0.0.1:18333",
            "AGENT_ROOM_CONTENT_S3_BUCKET": "agent-room-content",
            "AGENT_ROOM_CONTENT_S3_REGION": "us-east-1",
            "AGENT_ROOM_CONTENT_S3_ACCESS_KEY": required_value(
                values, "S3_ACCESS_KEY"
            ),
            "AGENT_ROOM_CONTENT_S3_SECRET_KEY": required_value(
                values, "S3_SECRET_KEY"
            ),
            "AGENT_ROOM_CONTENT_SCANNER_ADDRESS": "127.0.0.1:13310",
            "AGENT_ROOM_CONTENT_TICKET_KEY_ID": "local-v1",
            "AGENT_ROOM_CONTENT_TICKET_SECRET": required_value(
                values, "CONTENT_TICKET_SECRET"
            ),
            "AGENT_ROOM_CONTENT_MATRIX_AGENT_ID": required_value(
                values, "CONTENT_MATRIX_AGENT_ID"
            ),
            "AGENT_ROOM_OIDC_ISSUER_URL": (
                "http://127.0.0.1:18080/realms/agent-room"
            ),
            "AGENT_ROOM_OIDC_CLIENT_ID": "agent-room-web",
            "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID": "agent-room-bridge",
            "AGENT_ROOM_OIDC_CLIENT_SECRET": required_value(
                values, "KEYCLOAK_CLIENT_SECRET"
            ),
            "AGENT_ROOM_OIDC_REDIRECT_URL": (
                "https://api.agent-room.localhost:18443/auth/oidc/callback"
            ),
            "AGENT_ROOM_FRONTEND_ORIGIN": "https://app.agent-room.localhost:18443",
            "AGENT_ROOM_MATRIX_SERVER_NAME": "matrix.agent-room.localhost",
            "AGENT_ROOM_OTEL_EXPORT_TIMEOUT_MS": "5000",
            "AGENT_ROOM_LOG_FILTER": "agent_room_control_plane=info,sqlx=warn",
        }
    )
    if enable_telemetry:
        environment["AGENT_ROOM_OTLP_TRACES_ENDPOINT"] = (
            "http://127.0.0.1:14318/v1/traces"
        )
    else:
        environment.pop("AGENT_ROOM_OTLP_TRACES_ENDPOINT", None)
    return environment
