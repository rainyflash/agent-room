"""以最小权限创建并读取生产 Secret 文件。"""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import secrets
import stat
import time
from typing import Final
import uuid


SECRET_NAMES: Final = (
    "postgres_bootstrap_password",
    "agent_room_db_migration_password",
    "agent_room_db_runtime_password",
    "postgres_metrics_password",
    "synapse_db_password",
    "keycloak_db_password",
    "keycloak_admin_password",
    "keycloak_web_client_secret",
    "keycloak_matrix_client_secret",
    "synapse_registration_secret",
    "synapse_lifecycle_admin_password",
    "synapse_appservice_token",
    "synapse_appservice_hs_token",
    "synapse_macaroon_secret",
    "synapse_form_secret",
    "s3_access_key",
    "s3_secret_key",
    "content_ticket_secret",
    "account_deletion_receipt_secret",
    "worker_replication_secret",
    "alertmanager_webhook_token",
    "grafana_admin_password",
)
DERIVED_SECRET_NAMES: Final = (
    "migration_database_url",
    "synapse_lifecycle_admin_token",
)
MAX_SECRET_BYTES: Final = 4_096
SECRET_DIRECTORY_MODE: Final = 0o700
# Compose 非 Swarm 模式会直接 bind-mount 源文件；父目录负责宿主隔离，文件按 Docker Secret 语义只读。
CONTAINER_SECRET_FILE_MODE: Final = 0o444


class SecretStoreError(RuntimeError):
    """表示 Secret 目录不安全、缺失或损坏。"""


@dataclass(frozen=True, slots=True)
class SecretStore:
    directory: Path

    def initialize(self) -> None:
        self.directory.mkdir(mode=SECRET_DIRECTORY_MODE, parents=True, exist_ok=True)
        _restrict_permissions(self.directory, directory=True)
        for name in SECRET_NAMES:
            path = self.path(name)
            if path.exists():
                self.read(name)
                _restrict_permissions(path, directory=False)
                continue
            value = _generate_secret(name)
            _exclusive_write(path, value)
        agent_id = self.path("content_matrix_agent_id")
        if not agent_id.exists():
            _exclusive_write(agent_id, str(_uuid7()))
        else:
            uuid.UUID(self.read("content_matrix_agent_id"))

    def path(self, name: str) -> Path:
        if name not in {*SECRET_NAMES, *DERIVED_SECRET_NAMES, "content_matrix_agent_id"}:
            raise SecretStoreError(f"未知 Secret 名称：{name}。")
        return self.directory / name

    def read(self, name: str) -> str:
        path = self.path(name)
        try:
            raw = path.read_bytes()
        except FileNotFoundError as error:
            raise SecretStoreError(f"缺少 Secret 文件：{name}。") from error
        if not raw or len(raw) > MAX_SECRET_BYTES or b"\0" in raw:
            raise SecretStoreError(f"Secret 文件无效：{name}。")
        try:
            value = raw.decode("utf-8").rstrip("\r\n")
        except UnicodeDecodeError as error:
            raise SecretStoreError(f"Secret 文件不是 UTF-8：{name}。") from error
        if not value or "\n" in value or "\r" in value:
            raise SecretStoreError(f"Secret 文件必须只有一行：{name}。")
        return value

    def write_derived(self, name: str, value: str) -> None:
        if name not in DERIVED_SECRET_NAMES:
            raise SecretStoreError(f"{name} 不是可派生 Secret。")
        if (
            not value
            or "\n" in value
            or "\r" in value
            or len(value.encode("utf-8")) > MAX_SECRET_BYTES
        ):
            raise SecretStoreError(f"派生 Secret 无效：{name}。")
        _replace_secret(self.path(name), value)


def _generate_secret(name: str) -> str:
    if name == "s3_access_key":
        return f"ar{secrets.token_hex(12)}"
    return secrets.token_urlsafe(48)


def _exclusive_write(path: Path, value: str) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as stream:
            stream.write(value)
            stream.write("\n")
    except BaseException:
        path.unlink(missing_ok=True)
        raise
    _restrict_permissions(path, directory=False)


def _replace_secret(path: Path, value: str) -> None:
    expected = f"{value}\n".encode("utf-8")
    if path.exists():
        try:
            if path.read_bytes() == expected:
                _restrict_permissions(path, directory=False)
                return
        except OSError as error:
            raise SecretStoreError(f"无法读取派生 Secret：{path.name}。") from error

    temporary = path.with_suffix(".tmp")
    if temporary.exists():
        temporary.chmod(0o600)
        temporary.unlink()
    try:
        _exclusive_write(temporary, value)
        # Windows 不允许替换只读目标；父目录仍为 0700，替换完成后立即恢复只读。
        if path.exists():
            path.chmod(0o600)
        os.replace(temporary, path)
    except OSError as error:
        raise SecretStoreError(f"无法替换派生 Secret：{path.name}。") from error
    finally:
        if temporary.exists():
            temporary.chmod(0o600)
            temporary.unlink()
        if path.exists():
            _restrict_permissions(path, directory=False)


def _restrict_permissions(path: Path, *, directory: bool) -> None:
    mode = SECRET_DIRECTORY_MODE if directory else CONTAINER_SECRET_FILE_MODE
    try:
        path.chmod(mode)
    except OSError as error:
        raise SecretStoreError(f"无法收紧权限：{path.name}。") from error
    if os.name != "nt":
        actual = stat.S_IMODE(path.stat().st_mode)
        if actual != mode:
            raise SecretStoreError(f"权限不安全：{path.name}。")


def _uuid7() -> uuid.UUID:
    unix_milliseconds = int(time.time() * 1_000) & ((1 << 48) - 1)
    random_a = secrets.randbits(12)
    random_b = secrets.randbits(62)
    value = (
        (unix_milliseconds << 80)
        | (0x7 << 76)
        | (random_a << 64)
        | (0b10 << 62)
        | random_b
    )
    return uuid.UUID(int=value)
