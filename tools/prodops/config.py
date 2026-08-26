"""生产部署配置的领域模型与严格解析。"""

from __future__ import annotations

from dataclasses import dataclass
import ipaddress
import json
from pathlib import Path
from pathlib import PurePosixPath, PureWindowsPath
import re
from typing import Final
from urllib.parse import urlparse


SCHEMA_VERSION: Final = 1
DNS_NAME: Final = re.compile(
    r"^(?=.{1,253}\.?$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.?$"
)
PROJECT_NAME: Final = re.compile(r"^[a-z0-9][a-z0-9_-]{2,62}$")
DATABASE_MODES: Final = frozenset({"embedded", "external"})
OBJECT_STORE_MODES: Final = frozenset({"embedded", "external"})
TLS_MODES: Final = frozenset({"disable", "prefer", "require", "verify-ca", "verify-full"})


class DeploymentConfigError(ValueError):
    """表示部署配置不完整、类型错误或不安全。"""


@dataclass(frozen=True, slots=True)
class PublicEndpoints:
    server_name: str
    app_domain: str
    api_domain: str
    matrix_domain: str
    identity_domain: str
    acme_email: str

    @property
    def app_origin(self) -> str:
        return f"https://{self.app_domain}"

    @property
    def api_origin(self) -> str:
        return f"https://{self.api_domain}"

    @property
    def matrix_origin(self) -> str:
        return f"https://{self.matrix_domain}"

    @property
    def identity_origin(self) -> str:
        return f"https://{self.identity_domain}"


@dataclass(frozen=True, slots=True)
class DatabaseConfig:
    mode: str
    host: str
    port: int
    tls_mode: str
    control_database: str
    control_migration_user: str
    control_runtime_user: str
    synapse_database: str
    synapse_user: str
    identity_database: str
    identity_user: str


@dataclass(frozen=True, slots=True)
class ObjectStoreConfig:
    mode: str
    endpoint: str
    health_url: str
    bucket: str
    region: str


@dataclass(frozen=True, slots=True)
class CapacityConfig:
    control_plane_replicas: int
    synapse_workers: int


@dataclass(frozen=True, slots=True)
class BackupConfig:
    repository: str
    retention_days: int
    rpo_minutes: int
    provider_pitr_evidence_file: str | None

    @property
    def archive_timeout_seconds(self) -> int:
        return self.rpo_minutes * 60


@dataclass(frozen=True, slots=True)
class DeploymentConfig:
    schema_version: int
    project_name: str
    public: PublicEndpoints
    database: DatabaseConfig
    object_store: ObjectStoreConfig
    capacity: CapacityConfig
    backup: BackupConfig
    telemetry_enabled: bool

    @classmethod
    def from_mapping(cls, value: object) -> "DeploymentConfig":
        root = _mapping(value, "根配置")
        _reject_unknown(
            root,
            {
                "$schema",
                "schemaVersion",
                "projectName",
                "public",
                "database",
                "objectStore",
                "capacity",
                "backup",
                "telemetry",
            },
            "根配置",
        )
        schema_version = _integer(root, "schemaVersion")
        if schema_version != SCHEMA_VERSION:
            raise DeploymentConfigError(
                f"仅支持 schemaVersion={SCHEMA_VERSION}，收到 {schema_version}。"
            )
        project_name = _text(root, "projectName")
        if not PROJECT_NAME.fullmatch(project_name):
            raise DeploymentConfigError("projectName 必须是安全的 Compose 项目名。")

        return cls(
            schema_version=schema_version,
            project_name=project_name,
            public=_parse_public(root.get("public")),
            database=_parse_database(root.get("database")),
            object_store=_parse_object_store(root.get("objectStore")),
            capacity=_parse_capacity(root.get("capacity")),
            backup=_parse_backup(root.get("backup"), root.get("database")),
            telemetry_enabled=_parse_telemetry(root.get("telemetry")),
        )

    @property
    def compose_profiles(self) -> tuple[str, ...]:
        profiles: list[str] = []
        if self.database.mode == "embedded":
            profiles.append("embedded-database")
        if self.object_store.mode == "embedded":
            profiles.append("embedded-object-store")
        return tuple(profiles)


def load_deployment_config(path: Path) -> DeploymentConfig:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise DeploymentConfigError(f"部署配置不存在：{path}") from error
    except json.JSONDecodeError as error:
        raise DeploymentConfigError(
            f"部署配置不是有效 JSON：第 {error.lineno} 行第 {error.colno} 列。"
        ) from error
    return DeploymentConfig.from_mapping(value)


def _parse_public(value: object) -> PublicEndpoints:
    source = _mapping(value, "public")
    keys = {
        "serverName",
        "appDomain",
        "apiDomain",
        "matrixDomain",
        "identityDomain",
        "acmeEmail",
    }
    _reject_unknown(source, keys, "public")
    domains = {
        name: _dns_name(_text(source, name), f"public.{name}")
        for name in (
            "serverName",
            "appDomain",
            "apiDomain",
            "matrixDomain",
            "identityDomain",
        )
    }
    if len(set(domains.values())) != len(domains):
        raise DeploymentConfigError("public 中的服务域名必须彼此独立。")
    email = _text(source, "acmeEmail")
    if email.count("@") != 1 or len(email) > 254 or any(character.isspace() for character in email):
        raise DeploymentConfigError("public.acmeEmail 不是有效的 ACME 联系邮箱。")
    return PublicEndpoints(
        server_name=domains["serverName"],
        app_domain=domains["appDomain"],
        api_domain=domains["apiDomain"],
        matrix_domain=domains["matrixDomain"],
        identity_domain=domains["identityDomain"],
        acme_email=email,
    )


def _parse_database(value: object) -> DatabaseConfig:
    source = _mapping(value, "database")
    keys = {
        "mode",
        "host",
        "port",
        "tlsMode",
    }
    _reject_unknown(source, keys, "database")
    mode = _enum(source, "mode", DATABASE_MODES)
    default_host = "postgres" if mode == "embedded" else None
    host = _host(_optional_text(source, "host", default_host), "database.host")
    port = _optional_integer(source, "port", 5432)
    if not 1 <= port <= 65_535:
        raise DeploymentConfigError("database.port 必须在 1–65535 之间。")
    default_tls = "disable" if mode == "embedded" else "verify-full"
    tls_mode = _optional_enum(source, "tlsMode", TLS_MODES, default_tls)
    if mode == "external" and tls_mode in {"disable", "prefer"}:
        raise DeploymentConfigError("外部 PostgreSQL 必须使用 require、verify-ca 或 verify-full。")
    return DatabaseConfig(
        mode=mode,
        host=host,
        port=port,
        tls_mode=tls_mode,
        control_database="agent_room",
        control_migration_user="agent_room",
        control_runtime_user="agent_room_runtime",
        synapse_database="synapse",
        synapse_user="synapse",
        identity_database="keycloak",
        identity_user="identity",
    )


def _parse_object_store(value: object) -> ObjectStoreConfig:
    source = _mapping(value, "objectStore")
    keys = {"mode", "endpoint", "healthUrl", "bucket", "region"}
    _reject_unknown(source, keys, "objectStore")
    mode = _enum(source, "mode", OBJECT_STORE_MODES)
    endpoint = _optional_text(
        source,
        "endpoint",
        "http://object-store:8333" if mode == "embedded" else None,
    )
    health_url = _optional_text(
        source,
        "healthUrl",
        "http://object-store:9333/cluster/status" if mode == "embedded" else None,
    )
    return ObjectStoreConfig(
        mode=mode,
        endpoint=_http_url(endpoint, "objectStore.endpoint"),
        health_url=_http_url(health_url, "objectStore.healthUrl"),
        bucket=_dns_label(_optional_text(source, "bucket", "agent-room-content"), "objectStore.bucket"),
        region=_safe_token(_optional_text(source, "region", "us-east-1"), "objectStore.region"),
    )


def _parse_capacity(value: object) -> CapacityConfig:
    source = _mapping(value, "capacity")
    _reject_unknown(source, {"controlPlaneReplicas", "synapseWorkers"}, "capacity")
    control_plane_replicas = _optional_integer(source, "controlPlaneReplicas", 1)
    synapse_workers = _optional_integer(source, "synapseWorkers", 0)
    if not 1 <= control_plane_replicas <= 16:
        raise DeploymentConfigError("capacity.controlPlaneReplicas 必须在 1–16 之间。")
    if not 0 <= synapse_workers <= 8:
        raise DeploymentConfigError("capacity.synapseWorkers 必须在 0–8 之间。")
    return CapacityConfig(control_plane_replicas, synapse_workers)


def _parse_backup(value: object, database_value: object) -> BackupConfig:
    source = _mapping(value, "backup")
    _reject_unknown(
        source,
        {
            "repository",
            "retentionDays",
            "rpoMinutes",
            "providerPitrEvidenceFile",
        },
        "backup",
    )
    repository = _absolute_host_path(_text(source, "repository"), "backup.repository")
    retention_days = _optional_integer(source, "retentionDays", 30)
    rpo_minutes = _optional_integer(source, "rpoMinutes", 15)
    if not 7 <= retention_days <= 365:
        raise DeploymentConfigError("backup.retentionDays 必须在 7–365 之间。")
    if not 1 <= rpo_minutes <= 15:
        raise DeploymentConfigError("backup.rpoMinutes 必须在 1–15 之间。")

    database = _mapping(database_value, "database")
    database_mode = _enum(database, "mode", DATABASE_MODES)
    evidence = source.get("providerPitrEvidenceFile")
    if evidence is not None and not isinstance(evidence, str):
        raise DeploymentConfigError("backup.providerPitrEvidenceFile 必须是绝对 POSIX 路径。")
    evidence_path = (
        _absolute_host_path(evidence, "backup.providerPitrEvidenceFile")
        if isinstance(evidence, str)
        else None
    )
    if database_mode == "external" and evidence_path is None:
        raise DeploymentConfigError("外部 PostgreSQL 必须配置供应商 PITR 证据文件。")
    if database_mode == "embedded" and evidence_path is not None:
        raise DeploymentConfigError("内置 PostgreSQL 不接受外部 PITR 证据文件。")
    return BackupConfig(
        repository=repository,
        retention_days=retention_days,
        rpo_minutes=rpo_minutes,
        provider_pitr_evidence_file=evidence_path,
    )


def _parse_telemetry(value: object) -> bool:
    source = _mapping(value, "telemetry")
    _reject_unknown(source, {"enabled"}, "telemetry")
    enabled = source.get("enabled", True)
    if not isinstance(enabled, bool):
        raise DeploymentConfigError("telemetry.enabled 必须是布尔值。")
    return enabled


def _mapping(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise DeploymentConfigError(f"{label} 必须是 JSON 对象。")
    return value


def _reject_unknown(source: dict[str, object], allowed: set[str], label: str) -> None:
    unknown = sorted(set(source) - allowed)
    if unknown:
        raise DeploymentConfigError(f"{label} 包含未知字段：{', '.join(unknown)}。")


def _text(source: dict[str, object], name: str) -> str:
    value = source.get(name)
    if not isinstance(value, str) or not value.strip() or len(value) > 1_024:
        raise DeploymentConfigError(f"{name} 必须是非空短文本。")
    if any(ord(character) < 32 or ord(character) == 127 for character in value):
        raise DeploymentConfigError(f"{name} 不得包含控制字符。")
    return value.strip()


def _optional_text(source: dict[str, object], name: str, default: str | None) -> str:
    if name not in source:
        if default is None:
            raise DeploymentConfigError(f"缺少必需配置：{name}。")
        return default
    return _text(source, name)


def _integer(source: dict[str, object], name: str) -> int:
    value = source.get(name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise DeploymentConfigError(f"{name} 必须是整数。")
    return value


def _optional_integer(source: dict[str, object], name: str, default: int) -> int:
    return default if name not in source else _integer(source, name)


def _enum(source: dict[str, object], name: str, allowed: frozenset[str]) -> str:
    value = _text(source, name)
    if value not in allowed:
        raise DeploymentConfigError(f"{name} 仅支持：{', '.join(sorted(allowed))}。")
    return value


def _optional_enum(
    source: dict[str, object], name: str, allowed: frozenset[str], default: str
) -> str:
    if name not in source:
        return default
    return _enum(source, name, allowed)


def _dns_name(value: str, label: str) -> str:
    normalized = value.rstrip(".").lower()
    if not DNS_NAME.fullmatch(normalized):
        raise DeploymentConfigError(f"{label} 必须是规范 DNS 名称。")
    return normalized


def _dns_label(value: str, label: str) -> str:
    normalized = value.lower()
    if (
        not 3 <= len(normalized) <= 63
        or not re.fullmatch(r"[a-z0-9][a-z0-9.-]*[a-z0-9]", normalized)
        or ".." in normalized
    ):
        raise DeploymentConfigError(f"{label} 必须是安全的 S3 桶名。")
    return normalized


def _host(value: str, label: str) -> str:
    try:
        ipaddress.ip_address(value)
        return value
    except ValueError:
        pass
    if value == "postgres" or DNS_NAME.fullmatch(value.lower()):
        return value.lower()
    raise DeploymentConfigError(f"{label} 必须是 IP 或 DNS 主机名。")


def _http_url(value: str, label: str) -> str:
    parsed = urlparse(value)
    if (
        parsed.scheme not in {"http", "https"}
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
    ):
        raise DeploymentConfigError(f"{label} 必须是无凭据、查询和片段的 HTTP(S) URL。")
    return value.rstrip("/")


def _safe_token(value: str, label: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", value):
        raise DeploymentConfigError(f"{label} 格式无效。")
    return value


def _absolute_host_path(value: str, label: str) -> str:
    posix = PurePosixPath(value)
    windows = PureWindowsPath(value)
    if posix.is_absolute() and posix != PurePosixPath("/") and ".." not in posix.parts:
        return posix.as_posix()
    if windows.is_absolute() and windows.anchor != str(windows) and ".." not in windows.parts:
        return windows.as_posix()
    raise DeploymentConfigError(f"{label} 必须是非根绝对主机路径。")
