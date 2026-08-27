#!/usr/bin/env python3
"""幂等供应仅供账户生命周期 Worker 使用的 Synapse 管理员令牌。"""

from __future__ import annotations

import hashlib
import hmac
import json
import os
from pathlib import Path
import sys
from typing import Final
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen


BASE_URL: Final = "http://synapse:8008"
USERNAME: Final = "agent_room_lifecycle_admin"
DEVICE_NAME: Final = "Agent Room Account Lifecycle"
REGISTRATION_SECRET_FILE: Final = Path("/run/secrets/synapse_registration_secret")
PASSWORD_FILE: Final = Path("/run/secrets/synapse_lifecycle_admin_password")
TOKEN_FILE: Final = Path("/output/synapse_lifecycle_admin_token")
MAX_SECRET_BYTES: Final = 4_096


class BootstrapError(RuntimeError):
    """表示管理员账号或令牌无法安全供应。"""


def main() -> int:
    try:
        registration_secret = read_secret(REGISTRATION_SECRET_FILE)
        password = read_secret(PASSWORD_FILE)
        token, user_id = login(password)
        if token is None or user_id is None:
            token, user_id = register(registration_secret, password)
        assert_administrator(token, user_id)
        write_token(token)
    except BootstrapError as error:
        print(f"Synapse 生命周期管理员供应失败：{error}", file=sys.stderr)
        return 1
    print("Synapse 生命周期管理员令牌已安全供应。")
    return 0


def read_secret(path: Path) -> str:
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise BootstrapError(f"无法读取 Secret：{path.name}") from error
    if not raw or len(raw) > MAX_SECRET_BYTES or b"\0" in raw:
        raise BootstrapError(f"Secret 无效：{path.name}")
    try:
        value = raw.decode("utf-8").rstrip("\r\n")
    except UnicodeDecodeError as error:
        raise BootstrapError(f"Secret 不是 UTF-8：{path.name}") from error
    if not value or "\n" in value or "\r" in value:
        raise BootstrapError(f"Secret 必须只有一行：{path.name}")
    return value


def login(password: str) -> tuple[str | None, str | None]:
    status, body = request_json(
        "POST",
        "/_matrix/client/v3/login",
        {
            "type": "m.login.password",
            "identifier": {"type": "m.id.user", "user": USERNAME},
            "password": password,
            "initial_device_display_name": DEVICE_NAME,
        },
        accepted_statuses={200, 403},
    )
    if status == 403:
        return None, None
    return require_string(body, "access_token"), require_string(body, "user_id")


def register(registration_secret: str, password: str) -> tuple[str, str]:
    _, nonce_body = request_json("GET", "/_synapse/admin/v1/register")
    nonce = require_string(nonce_body, "nonce")
    mac = registration_mac(registration_secret, nonce, USERNAME, password, True)
    status, body = request_json(
        "POST",
        "/_synapse/admin/v1/register",
        {
            "nonce": nonce,
            "username": USERNAME,
            "password": password,
            "displayname": "Agent Room Lifecycle",
            "admin": True,
            "mac": mac,
        },
        accepted_statuses={200, 400},
    )
    if status == 400 and body.get("errcode") == "M_USER_IN_USE":
        raise BootstrapError("管理员已存在但生成的密码无法登录；拒绝覆盖未知账号")
    if status != 200:
        raise BootstrapError(
            f"共享密钥注册被 Synapse 拒绝{matrix_error_summary(body)}"
        )
    return require_string(body, "access_token"), require_string(body, "user_id")


def registration_mac(
    secret: str, nonce: str, username: str, password: str, administrator: bool
) -> str:
    message = b"\0".join(
        (
            nonce.encode("utf-8"),
            username.encode("utf-8"),
            password.encode("utf-8"),
            b"admin" if administrator else b"notadmin",
        )
    )
    return hmac.new(secret.encode("utf-8"), message, hashlib.sha1).hexdigest()


def assert_administrator(token: str, user_id: str) -> None:
    _, body = request_json(
        "GET",
        f"/_synapse/admin/v1/users/{quote(user_id, safe='')}/admin",
        token=token,
    )
    if body.get("admin") is not True:
        raise BootstrapError("供应账号没有 Synapse 管理员权限")


def write_token(token: str) -> None:
    if not token or len(token.encode("utf-8")) > MAX_SECRET_BYTES:
        raise BootstrapError("Synapse 返回了无效访问令牌")
    TOKEN_FILE.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = TOKEN_FILE.with_suffix(".tmp")
    temporary.unlink(missing_ok=True)
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as stream:
            stream.write(token)
            stream.write("\n")
        os.replace(temporary, TOKEN_FILE)
        TOKEN_FILE.chmod(0o444)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def request_json(
    method: str,
    path: str,
    payload: dict[str, object] | None = None,
    *,
    token: str | None = None,
    accepted_statuses: set[int] | None = None,
) -> tuple[int, dict[str, object]]:
    headers = {"Accept": "application/json"}
    data = None
    if payload is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    request = Request(f"{BASE_URL}{path}", data=data, headers=headers, method=method)
    expected = {200} if accepted_statuses is None else accepted_statuses
    try:
        with urlopen(request, timeout=15) as response:
            status = response.status
            raw = response.read(MAX_SECRET_BYTES * 4)
    except HTTPError as error:
        status = error.code
        raw = error.read(MAX_SECRET_BYTES * 4)
        if status not in expected:
            raise BootstrapError(f"Synapse 请求失败（HTTP {status}）") from error
    except (URLError, TimeoutError) as error:
        raise BootstrapError("无法连接内部 Synapse") from error
    if status not in expected:
        raise BootstrapError(f"Synapse 返回非预期状态（HTTP {status}）")
    try:
        parsed = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BootstrapError("Synapse 返回了无效 JSON") from error
    if not isinstance(parsed, dict):
        raise BootstrapError("Synapse 响应不是 JSON 对象")
    return status, parsed


def require_string(value: dict[str, object], field: str) -> str:
    candidate = value.get(field)
    if not isinstance(candidate, str) or not candidate:
        raise BootstrapError(f"Synapse 响应缺少 {field}")
    return candidate


def matrix_error_summary(value: dict[str, object]) -> str:
    """只保留 Matrix 错误码和单行短消息，避免把任意响应写进日志。"""
    details: list[str] = []
    for field in ("errcode", "error"):
        candidate = value.get(field)
        if not isinstance(candidate, str):
            continue
        normalized = " ".join(candidate.split())
        if normalized:
            details.append(normalized[:200])
    return f"（{'：'.join(details)}）" if details else ""


if __name__ == "__main__":
    raise SystemExit(main())
