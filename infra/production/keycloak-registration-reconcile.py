#!/usr/bin/env python3
"""以失败关闭策略同步 Agent Room 的 Keycloak 登录与注册配置。"""

from __future__ import annotations

import json
import os
from pathlib import Path
import smtplib
import ssl
import sys
from typing import Final
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode, urlsplit
from urllib.request import Request, urlopen


REALM: Final = "agent-room"
WEB_CLIENT_ID: Final = "agent-room-web"
ADMIN_USERNAME: Final = "agent-room-admin"
REQUEST_TIMEOUT_SECONDS: Final = 20
USER_IDENTITY_ACTION_LIFESPAN_SECONDS: Final = 60 * 60


class ReconcileError(RuntimeError):
    """表示身份配置无法安全同步。"""


def require_environment(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value or any(character in value for character in "\r\n\0"):
        raise ReconcileError(f"缺少或无效环境变量：{name}。")
    return value


def require_https_url(name: str, *, origin_only: bool) -> str:
    value = require_environment(name)
    parsed = urlsplit(value)
    invalid_path = origin_only and parsed.path not in {"", "/"}
    if (
        parsed.scheme != "https"
        or parsed.hostname is None
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or invalid_path
    ):
        expected_kind = "Origin" if origin_only else "URL"
        raise ReconcileError(f"{name} 必须是有效的 HTTPS {expected_kind}。")
    return value.rstrip("/") if origin_only else value


def read_secret(path: str, label: str) -> str:
    try:
        raw = Path(path).read_bytes()
    except OSError as error:
        raise ReconcileError(f"无法读取 {label} Secret。") from error
    if not raw or len(raw) > 4_096 or b"\0" in raw:
        raise ReconcileError(f"{label} Secret 无效。")
    try:
        value = raw.decode("utf-8").rstrip("\r\n")
    except UnicodeDecodeError as error:
        raise ReconcileError(f"{label} Secret 不是 UTF-8。") from error
    if not value or "\n" in value or "\r" in value:
        raise ReconcileError(f"{label} Secret 必须只有一行。")
    return value


def request(
    url: str,
    *,
    method: str = "GET",
    token: str | None = None,
    body: bytes | None = None,
    content_type: str | None = None,
) -> bytes:
    headers = {"Accept": "application/json"}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if content_type is not None:
        headers["Content-Type"] = content_type
    operation = Request(url, data=body, headers=headers, method=method)
    try:
        with urlopen(operation, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            return response.read()
    except HTTPError as error:
        raise ReconcileError(
            f"Keycloak 管理接口返回 HTTP {error.code}（{method}）。"
        ) from error
    except (URLError, TimeoutError, OSError) as error:
        raise ReconcileError(f"无法连接 Keycloak 管理接口（{method}）。") from error


def admin_token(base_url: str, password: str) -> str:
    payload = urlencode(
        {
            "grant_type": "password",
            "client_id": "admin-cli",
            "username": ADMIN_USERNAME,
            "password": password,
        }
    ).encode("ascii")
    raw = request(
        f"{base_url}/realms/master/protocol/openid-connect/token",
        method="POST",
        body=payload,
        content_type="application/x-www-form-urlencoded",
    )
    try:
        response = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ReconcileError("Keycloak 管理令牌响应不是有效 JSON。") from error
    if not isinstance(response, dict) or not isinstance(response.get("access_token"), str):
        raise ReconcileError("Keycloak 管理令牌响应缺少 access_token。")
    return response["access_token"]


def load_realm(base_url: str, token: str) -> dict[str, object]:
    raw = request(
        f"{base_url}/admin/realms/{quote(REALM, safe='')}",
        token=token,
    )
    try:
        realm = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ReconcileError("Keycloak Realm 响应不是有效 JSON。") from error
    if not isinstance(realm, dict):
        raise ReconcileError("Keycloak Realm 响应结构无效。")
    return realm


def load_web_client(base_url: str, token: str) -> dict[str, object]:
    raw = request(
        f"{base_url}/admin/realms/{quote(REALM, safe='')}/clients"
        f"?clientId={quote(WEB_CLIENT_ID, safe='')}",
        token=token,
    )
    try:
        clients = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ReconcileError("Keycloak Web Client 响应不是有效 JSON。") from error
    if not isinstance(clients, list) or len(clients) != 1 or not isinstance(clients[0], dict):
        raise ReconcileError("Keycloak Web Client 不存在或不唯一。")
    client_id = clients[0].get("id")
    if not isinstance(client_id, str) or not client_id:
        raise ReconcileError("Keycloak Web Client 缺少内部 ID。")
    detail_raw = request(
        f"{base_url}/admin/realms/{quote(REALM, safe='')}/clients/"
        f"{quote(client_id, safe='')}",
        token=token,
    )
    try:
        client = json.loads(detail_raw)
    except json.JSONDecodeError as error:
        raise ReconcileError("Keycloak Web Client 详情不是有效 JSON。") from error
    if not isinstance(client, dict) or client.get("id") != client_id:
        raise ReconcileError("Keycloak Web Client 详情结构无效。")
    return client


def apply_web_client_policy(
    client: dict[str, object],
    *,
    redirect_url: str,
    frontend_origin: str,
) -> dict[str, object]:
    updated = dict(client)
    updated.update(
        {
            "redirectUris": [redirect_url],
            "webOrigins": [frontend_origin],
            "standardFlowEnabled": True,
            "directAccessGrantsEnabled": False,
        }
    )
    return updated


def apply_registration_policy(
    realm: dict[str, object],
    *,
    enabled: bool,
    smtp: dict[str, str] | None,
) -> dict[str, object]:
    updated = dict(realm)
    updated.update(
        {
            "registrationAllowed": enabled,
            "registrationEmailAsUsername": True,
            "editUsernameAllowed": False,
            "duplicateEmailsAllowed": False,
            "loginWithEmailAllowed": True,
            "verifyEmail": True,
            "resetPasswordAllowed": True,
            "actionTokenGeneratedByUserLifespan": USER_IDENTITY_ACTION_LIFESPAN_SECONDS,
            "accessCodeLifespanUserAction": USER_IDENTITY_ACTION_LIFESPAN_SECONDS,
            "rememberMe": True,
            "bruteForceProtected": True,
        }
    )
    if smtp is None:
        updated.pop("smtpServer", None)
    else:
        updated["smtpServer"] = smtp
    return updated


def update_realm(base_url: str, token: str, realm: dict[str, object]) -> None:
    request(
        f"{base_url}/admin/realms/{quote(REALM, safe='')}",
        method="PUT",
        token=token,
        body=json.dumps(realm, ensure_ascii=False).encode("utf-8"),
        content_type="application/json",
    )


def update_web_client(base_url: str, token: str, client: dict[str, object]) -> None:
    client_id = client.get("id")
    if not isinstance(client_id, str) or not client_id:
        raise ReconcileError("Keycloak Web Client 缺少内部 ID。")
    request(
        f"{base_url}/admin/realms/{quote(REALM, safe='')}/clients/"
        f"{quote(client_id, safe='')}",
        method="PUT",
        token=token,
        body=json.dumps(client, ensure_ascii=False).encode("utf-8"),
        content_type="application/json",
    )


def smtp_settings(password: str) -> dict[str, str]:
    encryption = require_environment("AGENT_ROOM_SMTP_ENCRYPTION")
    if encryption not in {"starttls", "ssl"}:
        raise ReconcileError("SMTP 加密方式只允许 starttls 或 ssl。")
    return {
        "host": require_environment("AGENT_ROOM_SMTP_HOST"),
        "port": require_environment("AGENT_ROOM_SMTP_PORT"),
        "from": require_environment("AGENT_ROOM_SMTP_FROM_ADDRESS"),
        "fromDisplayName": require_environment("AGENT_ROOM_SMTP_FROM_DISPLAY_NAME"),
        "auth": "true",
        "user": require_environment("AGENT_ROOM_SMTP_USERNAME"),
        "password": password,
        "starttls": "true" if encryption == "starttls" else "false",
        "ssl": "true" if encryption == "ssl" else "false",
    }


def verify_smtp_transport(settings: dict[str, str]) -> None:
    try:
        port = int(settings["port"])
    except (KeyError, ValueError) as error:
        raise ReconcileError("SMTP 端口无效。") from error
    context = ssl.create_default_context()
    try:
        if settings["ssl"] == "true":
            with smtplib.SMTP_SSL(
                settings["host"],
                port,
                timeout=REQUEST_TIMEOUT_SECONDS,
                context=context,
            ) as client:
                client.login(settings["user"], settings["password"])
                client.noop()
        else:
            with smtplib.SMTP(
                settings["host"], port, timeout=REQUEST_TIMEOUT_SECONDS
            ) as client:
                client.ehlo()
                client.starttls(context=context)
                client.ehlo()
                client.login(settings["user"], settings["password"])
                client.noop()
    except (OSError, smtplib.SMTPException, ssl.SSLError) as error:
        raise ReconcileError("SMTP TLS 连接或身份验证失败，注册保持关闭。") from error


def reconcile() -> None:
    mode = require_environment("AGENT_ROOM_IDENTITY_REGISTRATION_MODE")
    if mode not in {"closed", "open-email"}:
        raise ReconcileError("身份注册模式无效。")
    base_url = require_environment("AGENT_ROOM_KEYCLOAK_INTERNAL_URL").rstrip("/")
    admin_password = read_secret(
        require_environment("AGENT_ROOM_KEYCLOAK_ADMIN_PASSWORD_FILE"),
        "Keycloak 管理员密码",
    )
    token = admin_token(base_url, admin_password)
    realm = load_realm(base_url, token)

    # 每次同步先关闭注册，后续任一步失败都不会留下半可用的注册入口。
    closed_realm = apply_registration_policy(realm, enabled=False, smtp=None)
    update_realm(base_url, token, closed_realm)
    redirect_url = require_https_url("AGENT_ROOM_OIDC_REDIRECT_URL", origin_only=False)
    frontend_origin = require_https_url("AGENT_ROOM_FRONTEND_ORIGIN", origin_only=True)
    web_client = load_web_client(base_url, token)
    update_web_client(
        base_url,
        token,
        apply_web_client_policy(
            web_client,
            redirect_url=redirect_url,
            frontend_origin=frontend_origin,
        ),
    )
    if mode == "closed":
        print("Keycloak 公开注册已关闭。")
        return

    smtp_password = read_secret(
        require_environment("AGENT_ROOM_SMTP_PASSWORD_FILE"),
        "SMTP 密码",
    )
    smtp = smtp_settings(smtp_password)
    verify_smtp_transport(smtp)
    open_realm = apply_registration_policy(closed_realm, enabled=True, smtp=smtp)
    update_realm(base_url, token, open_realm)
    print("Keycloak 邮箱验证注册已开放。")


def main() -> int:
    try:
        reconcile()
    except ReconcileError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
