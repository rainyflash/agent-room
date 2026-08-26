"""集中生成本地进程配置，避免每个启动器复制敏感环境装配。"""

from __future__ import annotations

from collections.abc import Mapping
import os
from pathlib import Path
import re
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"
MATRIX_LIFECYCLE_TOKEN_FILE: Final = (
    ROOT / ".local" / "secrets" / "synapse-lifecycle-admin-token"
)
SECURE_STORAGE_SERVICE: Final = re.compile(
    r"^[A-Za-z0-9](?:[A-Za-z0-9._-]{1,126}[A-Za-z0-9])?$"
)


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
    values: Mapping[str, str],
    *,
    enable_telemetry: bool,
    matrix_lifecycle_token_file: Path = MATRIX_LIFECYCLE_TOKEN_FILE,
) -> dict[str, str]:
    if not matrix_lifecycle_token_file.is_file():
        raise LocalRuntimeError("缺少 Matrix 生命周期管理令牌；请先运行 just dev-seed。")
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
            "AGENT_ROOM_MATRIX_ADMIN_ACCESS_TOKEN_FILE": str(
                matrix_lifecycle_token_file
            ),
            "AGENT_ROOM_ACCOUNT_DELETION_RECEIPT_SECRET": required_value(
                values, "ACCOUNT_DELETION_RECEIPT_SECRET"
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
        environment["AGENT_ROOM_OTLP_METRICS_ENDPOINT"] = (
            "http://127.0.0.1:14318/v1/metrics"
        )
    else:
        environment.pop("AGENT_ROOM_OTLP_TRACES_ENDPOINT", None)
        environment.pop("AGENT_ROOM_OTLP_METRICS_ENDPOINT", None)
    return environment


def bridge_runtime_environment(
    *,
    data_root: Path,
    agent_id: str | None = None,
    public_lobby_catalog_id: str | None = None,
    secure_storage_service: str | None = None,
    base_environment: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """生成 Bridge 唯一可信的本地运行时环境。"""
    if not data_root.is_absolute():
        raise LocalRuntimeError("Bridge 数据目录必须是绝对路径。")
    if (agent_id is None) != (public_lobby_catalog_id is None):
        raise LocalRuntimeError("Agent 与公共大厅目录必须同时配置。")
    if secure_storage_service is not None and not SECURE_STORAGE_SERVICE.fullmatch(
        secure_storage_service
    ):
        raise LocalRuntimeError("Bridge 安全存储命名空间格式无效。")

    environment = dict(os.environ if base_environment is None else base_environment)
    environment.update(
        {
            "AGENT_ROOM_CONTROL_PLANE_URL": "http://127.0.0.1:8090/",
            "AGENT_ROOM_MATRIX_BASE_URL": "http://127.0.0.1:18008",
            "AGENT_ROOM_OIDC_ISSUER_URL": (
                "http://127.0.0.1:18080/realms/agent-room"
            ),
            "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID": "agent-room-bridge",
            "AGENT_ROOM_BRIDGE_REQUEST_TIMEOUT_MS": "10000",
            "AGENT_ROOM_BRIDGE_AUTHORIZATION_TIMEOUT_MS": "600000",
            "AGENT_ROOM_BRIDGE_REFRESH_LEAD_MS": "120000",
            "AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS": "1000",
            "AGENT_ROOM_BRIDGE_RECONNECT_MAXIMUM_MS": "60000",
            "AGENT_ROOM_BRIDGE_MATRIX_SYNC_TIMEOUT_MS": "1000",
            "AGENT_ROOM_BRIDGE_DATA_DIR": str(data_root),
            "AGENT_ROOM_BRIDGE_IMPORT_OIDC_PROFILE": "false",
            "AGENT_ROOM_BRIDGE_DEVICE_LABEL": "Agent Room 本地 Bridge",
        }
    )
    _replace_optional(environment, "AGENT_ROOM_AGENT_ID", agent_id)
    _replace_optional(
        environment,
        "AGENT_ROOM_PUBLIC_LOBBY_CATALOG_ID",
        public_lobby_catalog_id,
    )
    _replace_optional(environment, "AGENT_ROOM_LOBBY_LANGUAGE", "en" if agent_id else None)
    _replace_optional(environment, "AGENT_ROOM_LOBBY_REGION", None)
    _replace_optional(
        environment,
        "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE",
        secure_storage_service,
    )
    return environment


def _replace_optional(
    environment: dict[str, str], name: str, value: str | None
) -> None:
    if value is None:
        environment.pop(name, None)
        return
    environment[name] = value
