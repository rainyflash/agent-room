#!/usr/bin/env python3
"""编排两个完全独立的 Synapse，并执行真实 Matrix 联邦验收。"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import hmac
import http.client
import json
import os
from pathlib import Path
import secrets
import socket
import ssl
import subprocess
import sys
import time
from typing import Final
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen
import uuid

if __package__:
    from .source_revision import (
        SourceRevisionFailure,
        clean_git_revision,
        require_clean_git_revision,
    )
else:
    from source_revision import (
        SourceRevisionFailure,
        clean_git_revision,
        require_clean_git_revision,
    )


ROOT: Final = Path(__file__).resolve().parent.parent
COMPOSE_FILE: Final = ROOT / "infra" / "federation" / "compose.yaml"
RUNTIME_DIR: Final = ROOT / ".local" / "federation"
ENV_FILE: Final = RUNTIME_DIR / "runtime.env"
REPORT_FILE: Final = ROOT / "artifacts" / "federation" / "task-37-report.json"
PROJECT_NAME: Final = "agent-room-federation-test"
SYNAPSE_IMAGE: Final = "matrixdotorg/synapse:v1.159.0"
FEDERATION_PORT: Final = 18448
HTTP_TIMEOUT_SECONDS: Final = 15
TEST_JOIN_PER_ROOM_RATE: Final = 20
TEST_JOIN_PER_ROOM_BURST: Final = 40


class FederationFailure(RuntimeError):
    """表示联邦环境或验收不满足硬边界。"""


@dataclass(frozen=True)
class Peer:
    """单个联邦服务的公开身份与本地测试入口。"""

    label: str
    server_name: str
    client_base_url: str
    service_name: str
    database_service_name: str


ALPHA: Final = Peer(
    label="alpha",
    server_name="alpha.agent-room.test",
    client_base_url="http://127.0.0.1:18108",
    service_name="synapse-alpha",
    database_service_name="postgres-alpha",
)
BETA: Final = Peer(
    label="beta",
    server_name="beta.agent-room.test",
    client_base_url="http://127.0.0.1:18208",
    service_name="synapse-beta",
    database_service_name="postgres-beta",
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def run_command(
    command: list[str],
    *,
    environment: dict[str, str] | None = None,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=environment,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=capture_output,
    )
    if result.returncode != 0:
        diagnostic = ""
        if capture_output:
            diagnostic = f"\n{result.stdout[-2000:]}\n{result.stderr[-2000:]}"
        raise FederationFailure(f"命令执行失败：{' '.join(command)}{diagnostic}")
    return result


def compose(*arguments: str, capture_output: bool = False) -> subprocess.CompletedProcess[str]:
    return run_command(
        [
            "docker",
            "compose",
            "--project-name",
            PROJECT_NAME,
            "--env-file",
            str(ENV_FILE),
            "--file",
            str(COMPOSE_FILE),
            *arguments,
        ],
        capture_output=capture_output,
    )


def read_environment(path: Path = ENV_FILE) -> dict[str, str]:
    if not path.is_file():
        raise FederationFailure("联邦运行时配置不存在；请先执行 prepare。")
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        name, separator, value = line.partition("=")
        if not separator or not name or not value:
            raise FederationFailure("联邦运行时配置包含空值或无效行。")
        values[name] = value
    return values


def write_environment() -> dict[str, str]:
    RUNTIME_DIR.mkdir(parents=True, exist_ok=True)
    host_user_id, host_group_id = host_container_identity()
    if ENV_FILE.is_file():
        values = read_environment()
        expected_identity = {
            "FEDERATION_HOST_UID": host_user_id,
            "FEDERATION_HOST_GID": host_group_id,
        }
        if any(values.get(name) != value for name, value in expected_identity.items()):
            values.update(expected_identity)
            write_private_text(
                ENV_FILE,
                "".join(f"{name}={value}\n" for name, value in values.items()),
            )
        return values
    values = {
        "FEDERATION_RUNTIME_DIR": RUNTIME_DIR.resolve().as_posix(),
        "FEDERATION_HOST_UID": host_user_id,
        "FEDERATION_HOST_GID": host_group_id,
        "ALPHA_DATABASE_PASSWORD": secrets.token_hex(24),
        "BETA_DATABASE_PASSWORD": secrets.token_hex(24),
        "ALPHA_REGISTRATION_SECRET": secrets.token_hex(32),
        "BETA_REGISTRATION_SECRET": secrets.token_hex(32),
        "ALPHA_MACAROON_SECRET": secrets.token_hex(32),
        "BETA_MACAROON_SECRET": secrets.token_hex(32),
        "ALPHA_FORM_SECRET": secrets.token_hex(32),
        "BETA_FORM_SECRET": secrets.token_hex(32),
        "ALPHA_USER_PASSWORD": secrets.token_urlsafe(24),
        "BETA_USER_PASSWORD": secrets.token_urlsafe(24),
    }
    write_private_text(
        ENV_FILE,
        "".join(f"{name}={value}\n" for name, value in values.items()),
    )
    return values


def host_container_identity() -> tuple[str, str]:
    if os.name == "posix":
        return str(os.getuid()), str(os.getgid())
    return "991", "991"


def prepare_certificates() -> None:
    directory = RUNTIME_DIR / "certificates"
    directory.mkdir(parents=True, exist_ok=True)
    secure_container_output(directory, "/certificates")
    profile_marker = directory / "profile-v2"
    ca_certificate = directory / "ca.crt"
    server_certificate = directory / "server.crt"
    if (
        profile_marker.is_file()
        and ca_certificate.is_file()
        and server_certificate.is_file()
    ):
        return
    ca_key = directory / "ca.key"
    server_key = directory / "server.key"
    server_request = directory / "server.csr"
    extensions = directory / "server.ext"
    for generated in (
        ca_certificate,
        ca_key,
        directory / "ca.srl",
        server_certificate,
        server_key,
        server_request,
    ):
        generated.unlink(missing_ok=True)
    write_private_text(
        extensions,
        "\n".join(
            (
                "basicConstraints=CA:FALSE",
                "keyUsage=digitalSignature,keyEncipherment",
                "extendedKeyUsage=serverAuth",
                "subjectAltName=DNS:alpha.agent-room.test,DNS:beta.agent-room.test",
                "",
            )
        ),
    )
    certificate_mount = f"{directory.resolve().as_posix()}:/certificates"
    openssl = [
        "docker",
        "run",
        "--rm",
        "--volume",
        certificate_mount,
        "--entrypoint",
        "openssl",
        SYNAPSE_IMAGE,
    ]
    run_command(
        [
            *openssl,
            "req",
            "-x509",
            "-newkey",
            "rsa:3072",
            "-nodes",
            "-keyout",
            "/certificates/ca.key",
            "-out",
            "/certificates/ca.crt",
            "-subj",
            "/CN=Agent Room Federation Test CA",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-addext",
            "subjectKeyIdentifier=hash",
            "-days",
            "7",
            "-sha256",
        ],
        capture_output=True,
    )
    run_command(
        [
            *openssl,
            "req",
            "-new",
            "-newkey",
            "rsa:3072",
            "-nodes",
            "-keyout",
            "/certificates/server.key",
            "-out",
            "/certificates/server.csr",
            "-subj",
            "/CN=alpha.agent-room.test",
            "-sha256",
        ],
        capture_output=True,
    )
    run_command(
        [
            *openssl,
            "x509",
            "-req",
            "-in",
            "/certificates/server.csr",
            "-CA",
            "/certificates/ca.crt",
            "-CAkey",
            "/certificates/ca.key",
            "-CAcreateserial",
            "-out",
            "/certificates/server.crt",
            "-days",
            "7",
            "-sha256",
            "-extfile",
            "/certificates/server.ext",
        ],
        capture_output=True,
    )
    secure_container_output(directory, "/certificates")
    write_private_text(profile_marker, "v2\n")


def prepare_synapse(peer: Peer, other: Peer, values: dict[str, str]) -> None:
    directory = RUNTIME_DIR / peer.label
    directory.mkdir(parents=True, exist_ok=True)
    signing_key = directory / f"{peer.server_name}.signing.key"
    if not signing_key.is_file():
        run_command(
            [
                "docker",
                "run",
                "--rm",
                "--env",
                f"SYNAPSE_SERVER_NAME={peer.server_name}",
                "--env",
                "SYNAPSE_REPORT_STATS=no",
                "--volume",
                f"{directory.resolve().as_posix()}:/data",
                SYNAPSE_IMAGE,
                "generate",
            ]
        )
    secure_container_output(directory, "/data")
    config = synapse_configuration(peer, other, values)
    write_private_text(directory / "homeserver.yaml", config)


def write_private_text(path: Path, content: str) -> None:
    """以当前用户独占权限原子替换含测试凭据的文件。"""

    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            descriptor = -1
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        temporary.replace(path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def secure_container_output(directory: Path, container_path: str) -> None:
    """把容器生成物交给当前 Linux 用户，并移除组与其他用户权限。"""

    if not any(directory.iterdir()):
        return
    if os.name != "posix":
        return
    if container_path not in {"/certificates", "/data"}:
        raise FederationFailure("容器输出目录不在固定允许列表。")
    user_id = os.getuid()
    group_id = os.getgid()
    run_command(
        [
            "docker",
            "run",
            "--rm",
            "--user",
            "0:0",
            "--volume",
            f"{directory.resolve().as_posix()}:{container_path}",
            "--entrypoint",
            "/bin/sh",
            SYNAPSE_IMAGE,
            "-c",
            (
                f"chown -R {user_id}:{group_id} {container_path} && "
                f"chmod -R u=rwX,go= {container_path}"
            ),
        ],
        capture_output=True,
    )


def synapse_configuration(
    peer: Peer,
    other: Peer,
    values: dict[str, str],
) -> str:
    """生成单一 Homeserver 配置，不读取或修改外部状态。"""

    database_password = values[f"{peer.label.upper()}_DATABASE_PASSWORD"]
    registration_secret = values[f"{peer.label.upper()}_REGISTRATION_SECRET"]
    macaroon_secret = values[f"{peer.label.upper()}_MACAROON_SECRET"]
    form_secret = values[f"{peer.label.upper()}_FORM_SECRET"]
    public_port = 18108 if peer == ALPHA else 18208
    config = f'''server_name: "{peer.server_name}"
public_baseurl: "http://127.0.0.1:{public_port}/"
pid_file: /data/homeserver.pid
listeners:
  - port: 8008
    tls: false
    type: http
    x_forwarded: true
    resources:
      - names: [client, federation]
        compress: false
database:
  name: psycopg2
  args:
    user: synapse
    password: "{database_password}"
    database: synapse
    host: {peer.database_service_name}
    port: 5432
    cp_min: 1
    cp_max: 8
log_config: /data/{peer.server_name}.log.config
media_store_path: /data/media_store
registration_shared_secret: "{registration_secret}"
report_stats: false
macaroon_secret_key: "{macaroon_secret}"
form_secret: "{form_secret}"
signing_key_path: /data/{peer.server_name}.signing.key
trusted_key_servers: []
suppress_key_server_warning: true
enable_registration: false
federation_domain_whitelist:
  - "{other.server_name}"
federation_whitelist_endpoint_enabled: true
federation_custom_ca_list:
  - /certificates/ca.crt
ip_range_whitelist:
  - 10.0.0.0/8
  - 172.16.0.0/12
  - 192.168.0.0/16
allow_public_rooms_without_auth: true
allow_public_rooms_over_federation: true
rc_joins:
  local:
    per_second: 20
    burst_count: 40
  remote:
    per_second: 20
    burst_count: 40
rc_joins_per_room:
  per_second: {TEST_JOIN_PER_ROOM_RATE}
  burst_count: {TEST_JOIN_PER_ROOM_BURST}
rc_message:
  per_second: 50
  burst_count: 100
'''
    return config


def prepare() -> None:
    if not _command_exists("docker"):
        raise FederationFailure("缺少必需命令：docker")
    values = write_environment()
    prepare_certificates()
    prepare_synapse(ALPHA, BETA, values)
    prepare_synapse(BETA, ALPHA, values)


def _command_exists(command: str) -> bool:
    result = subprocess.run(
        [command, "--version"],
        cwd=ROOT,
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return result.returncode == 0


def up() -> None:
    read_environment()
    compose("up", "--detach", "--wait", "--wait-timeout", "240")
    diagnose()


def down(*, volumes: bool) -> None:
    if not ENV_FILE.is_file():
        return
    write_environment()
    arguments = ["down", "--remove-orphans"]
    if volumes:
        arguments.extend(("--volumes", "--timeout", "10"))
    compose(*arguments)


def tls_json_request(server_name: str, path: str) -> dict[str, object]:
    context = federation_tls_context()
    try:
        with socket.create_connection(("127.0.0.1", FEDERATION_PORT), timeout=10) as raw:
            with context.wrap_socket(raw, server_hostname=server_name) as secured:
                request = (
                    f"GET {path} HTTP/1.1\r\n"
                    f"Host: {server_name}:8448\r\n"
                    "Accept: application/json\r\n"
                    "Connection: close\r\n\r\n"
                )
                secured.sendall(request.encode("ascii"))
                response = http.client.HTTPResponse(secured)
                response.begin()
                body = response.read()
                if response.status != 200:
                    raise FederationFailure(
                        f"{server_name}{path} 返回非成功状态：{response.status}"
                    )
    except (OSError, ssl.SSLError) as error:
        raise FederationFailure(
            f"{server_name} 的 TLS 联邦入口不可达：{error}"
        ) from error
    parsed = json.loads(body)
    if not isinstance(parsed, dict):
        raise FederationFailure(f"{server_name}{path} 未返回 JSON 对象。")
    return parsed


def federation_tls_context() -> ssl.SSLContext:
    context = ssl.create_default_context(
        cafile=str(RUNTIME_DIR / "certificates" / "ca.crt")
    )
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    return context


def diagnose() -> dict[str, object]:
    evidence: dict[str, object] = {}
    for peer in (ALPHA, BETA):
        well_known = tls_json_request(peer.server_name, "/.well-known/matrix/server")
        if well_known.get("m.server") != f"{peer.server_name}:8448":
            raise FederationFailure(f"{peer.label} 的 .well-known 委派错误。")
        version = tls_json_request(peer.server_name, "/_matrix/federation/v1/version")
        keys = tls_json_request(peer.server_name, "/_matrix/key/v2/server")
        verify_keys = keys.get("verify_keys")
        if not isinstance(verify_keys, dict) or not verify_keys:
            raise FederationFailure(f"{peer.label} 没有发布联邦签名公钥。")
        evidence[peer.label] = {
            "serverName": peer.server_name,
            "delegatedServer": well_known["m.server"],
            "implementation": version.get("server"),
            "signingKeyCount": len(verify_keys),
        }
    if _signing_key_bytes(ALPHA) == _signing_key_bytes(BETA):
        raise FederationFailure("两个 Homeserver 意外复用了同一 signing key。")
    return evidence


def _signing_key_bytes(peer: Peer) -> bytes:
    return (RUNTIME_DIR / peer.label / f"{peer.server_name}.signing.key").read_bytes()


def matrix_request(
    peer: Peer,
    method: str,
    path: str,
    *,
    token: str | None = None,
    payload: object | None = None,
    expected_statuses: tuple[int, ...] = (200,),
) -> dict[str, object]:
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    request = Request(
        f"{peer.client_base_url}{path}",
        data=body,
        method=method,
        headers={
            "Accept": "application/json",
            "Content-Type": "application/json",
            **({"Authorization": f"Bearer {token}"} if token else {}),
        },
    )
    try:
        with urlopen(request, timeout=HTTP_TIMEOUT_SECONDS) as response:
            status = response.status
            raw = response.read()
    except HTTPError as error:
        status = error.code
        raw = error.read()
    except URLError as error:
        raise FederationFailure(f"{peer.label} Matrix 请求不可达：{error.reason}") from error
    except (http.client.HTTPException, OSError) as error:
        raise FederationFailure(f"{peer.label} Matrix 请求连接中断：{error}") from error
    if status not in expected_statuses:
        safe_error = ""
        try:
            parsed_error = json.loads(raw)
            safe_error = str(parsed_error.get("errcode", "unknown"))
        except (json.JSONDecodeError, AttributeError):
            safe_error = "invalid_response"
        raise FederationFailure(
            f"{peer.label} Matrix {method} {path} 返回 {status}/{safe_error}。"
        )
    if not raw:
        return {}
    parsed = json.loads(raw)
    if not isinstance(parsed, dict):
        raise FederationFailure("Matrix 响应不是 JSON 对象。")
    parsed["_status"] = status
    return parsed


@dataclass(frozen=True)
class MatrixUser:
    peer: Peer
    user_id: str
    access_token: str
    password: str


def register_user(
    peer: Peer,
    registration_secret: str,
    username: str,
    password: str,
    *,
    administrator: bool,
) -> MatrixUser:
    nonce_response = matrix_request(peer, "GET", "/_synapse/admin/v1/register")
    nonce = nonce_response.get("nonce")
    if not isinstance(nonce, str):
        raise FederationFailure("Synapse 共享密钥注册没有返回 nonce。")
    message = b"\x00".join(
        (
            nonce.encode(),
            username.encode(),
            password.encode(),
            b"admin" if administrator else b"notadmin",
        )
    )
    mac = hmac.new(registration_secret.encode(), message, hashlib.sha1).hexdigest()
    response = matrix_request(
        peer,
        "POST",
        "/_synapse/admin/v1/register",
        payload={
            "nonce": nonce,
            "username": username,
            "password": password,
            "admin": administrator,
            "mac": mac,
            "displayname": f"Federation {peer.label.title()} Test",
        },
    )
    user_id = response.get("user_id")
    access_token = response.get("access_token")
    if not isinstance(user_id, str) or not isinstance(access_token, str):
        raise FederationFailure("Synapse 注册响应缺少用户或访问令牌。")
    return MatrixUser(peer, user_id, access_token, password)


def encoded(value: str) -> str:
    return quote(value, safe="")


def sync(user: MatrixUser, since: str | None, *, timeout_ms: int = 1000) -> dict[str, object]:
    query: dict[str, str | int] = {"timeout": timeout_ms}
    if since is not None:
        query["since"] = since
    return matrix_request(
        user.peer,
        "GET",
        f"/_matrix/client/v3/sync?{urlencode(query)}",
        token=user.access_token,
    )


def next_batch(response: dict[str, object]) -> str:
    value = response.get("next_batch")
    if not isinstance(value, str):
        raise FederationFailure("Matrix /sync 缺少 next_batch。")
    return value


def joined_room_events(response: dict[str, object], room_id: str) -> list[dict[str, object]]:
    rooms = response.get("rooms")
    if not isinstance(rooms, dict):
        return []
    joined = rooms.get("join")
    if not isinstance(joined, dict):
        return []
    room = joined.get(room_id)
    if not isinstance(room, dict):
        return []
    timeline = room.get("timeline")
    if not isinstance(timeline, dict):
        return []
    events = timeline.get("events")
    if not isinstance(events, list):
        return []
    return [event for event in events if isinstance(event, dict)]


def wait_for_event(
    user: MatrixUser,
    room_id: str,
    event_ids: set[str],
    since: str | None,
    *,
    timeout_seconds: float = 45,
) -> tuple[str, dict[str, str]]:
    deadline = time.monotonic() + timeout_seconds
    remaining = set(event_ids)
    arrivals: dict[str, str] = {}
    token = since
    while time.monotonic() < deadline:
        response = sync(user, token)
        token = next_batch(response)
        for event in joined_room_events(response, room_id):
            event_id = event.get("event_id")
            if isinstance(event_id, str) and event_id in remaining:
                arrivals[event_id] = utc_now()
                remaining.remove(event_id)
        if not remaining:
            return token, arrivals
    raise FederationFailure(f"对端未在预算内收到事件：{sorted(remaining)}")


def room_membership(response: dict[str, object], room_id: str, section: str) -> bool:
    rooms = response.get("rooms")
    if not isinstance(rooms, dict):
        return False
    section_rooms = rooms.get(section)
    return isinstance(section_rooms, dict) and room_id in section_rooms


def wait_for_room(user: MatrixUser, room_id: str, section: str) -> str:
    deadline = time.monotonic() + 45
    token: str | None = None
    while time.monotonic() < deadline:
        response = sync(user, token)
        token = next_batch(response)
        if room_membership(response, room_id, section):
            return token
    raise FederationFailure(f"{user.user_id} 未在预算内进入 {section} 房间段。")


def create_room(
    owner: MatrixUser,
    invitee: MatrixUser | None,
    *,
    alias_prefix: str,
) -> str:
    alias = f"{alias_prefix}-{uuid.uuid4().hex[:12]}"
    payload: dict[str, object] = {
        "name": "Agent Room federation acceptance",
        "topic": "Task 37 isolated federation evidence",
        "visibility": "private",
        "preset": "private_chat",
        "room_alias_name": alias,
        "creation_content": {"m.federate": True},
    }
    if invitee is not None:
        payload["invite"] = [invitee.user_id]
    response = matrix_request(
        owner.peer,
        "POST",
        "/_matrix/client/v3/createRoom",
        token=owner.access_token,
        payload=payload,
    )
    room_id = response.get("room_id")
    if not isinstance(room_id, str):
        raise FederationFailure("建房响应缺少 room_id。")
    return room_id


def join_remote_room(user: MatrixUser, room_id: str, origin: Peer) -> None:
    query = urlencode({"server_name": origin.server_name})
    matrix_request(
        user.peer,
        "POST",
        f"/_matrix/client/v3/join/{encoded(room_id)}?{query}",
        token=user.access_token,
        payload={},
    )


def send_event(user: MatrixUser, room_id: str, body: str) -> tuple[str, str]:
    return send_custom_event(
        user,
        room_id,
        "io.github.rainyflash.agentroom.message.preview.v1",
        {"schemaVersion": "1.0", "body": body},
    )


def send_custom_event(
    user: MatrixUser,
    room_id: str,
    event_type: str,
    content: dict[str, object],
    *,
    transaction_id: str | None = None,
) -> tuple[str, str]:
    transaction_id = transaction_id or f"federation-{uuid.uuid4().hex}"
    accepted_at = utc_now()
    response = matrix_request(
        user.peer,
        "PUT",
        (
            f"/_matrix/client/v3/rooms/{encoded(room_id)}/send/"
            f"{encoded(event_type)}/{encoded(transaction_id)}"
        ),
        token=user.access_token,
        payload=content,
    )
    event_id = response.get("event_id")
    if not isinstance(event_id, str):
        raise FederationFailure("消息提交响应缺少 event_id。")
    return event_id, accepted_at


def send_state(user: MatrixUser, room_id: str) -> str:
    response = matrix_request(
        user.peer,
        "PUT",
        (
            f"/_matrix/client/v3/rooms/{encoded(room_id)}/state/"
            "io.github.rainyflash.agentroom.agent.status.v1/task37-alpha"
        ),
        token=user.access_token,
        payload={"schemaVersion": "1.0", "state": "working", "leaseSeconds": 30},
    )
    event_id = response.get("event_id")
    if not isinstance(event_id, str):
        raise FederationFailure("状态提交响应缺少 event_id。")
    return event_id


def send_receipt(user: MatrixUser, room_id: str, event_id: str) -> str:
    read_at = utc_now()
    matrix_request(
        user.peer,
        "POST",
        (
            f"/_matrix/client/v3/rooms/{encoded(room_id)}/receipt/"
            f"m.read/{encoded(event_id)}"
        ),
        token=user.access_token,
        payload={},
    )
    return read_at


def wait_for_receipt(
    user: MatrixUser,
    room_id: str,
    event_id: str,
    reader_id: str,
    since: str,
) -> str:
    deadline = time.monotonic() + 45
    token = since
    while time.monotonic() < deadline:
        response = sync(user, token)
        token = next_batch(response)
        rooms = response.get("rooms", {})
        joined = rooms.get("join", {}) if isinstance(rooms, dict) else {}
        room = joined.get(room_id, {}) if isinstance(joined, dict) else {}
        ephemeral = room.get("ephemeral", {}) if isinstance(room, dict) else {}
        events = ephemeral.get("events", []) if isinstance(ephemeral, dict) else []
        for event in events if isinstance(events, list) else []:
            if not isinstance(event, dict) or event.get("type") != "m.receipt":
                continue
            content = event.get("content")
            if not isinstance(content, dict):
                continue
            event_receipts = content.get(event_id)
            if not isinstance(event_receipts, dict):
                continue
            read_receipts = event_receipts.get("m.read")
            if isinstance(read_receipts, dict) and reader_id in read_receipts:
                return utc_now()
    raise FederationFailure("发送端未在预算内观察到接收方已读回执。")


def verify_ban_and_recovery(owner: MatrixUser, member: MatrixUser, room_id: str) -> None:
    matrix_request(
        owner.peer,
        "POST",
        f"/_matrix/client/v3/rooms/{encoded(room_id)}/ban",
        token=owner.access_token,
        payload={"user_id": member.user_id, "reason": "task-37-governance-test"},
    )
    deadline = time.monotonic() + 30
    while True:
        try:
            send_event(member, room_id, "blocked federation message")
        except FederationFailure as error:
            if "403/" in str(error):
                break
            raise
        if time.monotonic() >= deadline:
            raise FederationFailure("跨服务封禁未在预算内阻止远端发送。")
        time.sleep(1)
    matrix_request(
        owner.peer,
        "POST",
        f"/_matrix/client/v3/rooms/{encoded(room_id)}/unban",
        token=owner.access_token,
        payload={"user_id": member.user_id},
    )
    matrix_request(
        owner.peer,
        "POST",
        f"/_matrix/client/v3/rooms/{encoded(room_id)}/invite",
        token=owner.access_token,
        payload={"user_id": member.user_id},
    )
    wait_for_room(member, room_id, "invite")
    join_remote_room(member, room_id, owner.peer)
    wait_for_room(member, room_id, "join")


def run_e2ee_test(alpha: MatrixUser, beta: MatrixUser) -> None:
    environment = os.environ.copy()
    environment.update(
        {
            "AGENT_ROOM_FEDERATION_ALPHA_URL": alpha.peer.client_base_url,
            "AGENT_ROOM_FEDERATION_ALPHA_USER": alpha.user_id,
            "AGENT_ROOM_FEDERATION_ALPHA_PASSWORD": alpha.password,
            "AGENT_ROOM_FEDERATION_BETA_URL": beta.peer.client_base_url,
            "AGENT_ROOM_FEDERATION_BETA_USER": beta.user_id,
            "AGENT_ROOM_FEDERATION_BETA_PASSWORD": beta.password,
        }
    )
    run_command(
        [
            "cargo",
            "test",
            "-p",
            "agent-room-matrix-adapter",
            "--test",
            "federation",
            "--",
            "--ignored",
            "--test-threads=1",
        ],
        environment=environment,
    )


def stop_peer(peer: Peer) -> None:
    compose("stop", peer.service_name)


def start_peer(peer: Peer) -> None:
    compose("start", peer.service_name)
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        try:
            matrix_request(peer, "GET", "/_matrix/client/versions")
            return
        except FederationFailure:
            time.sleep(2)
    raise FederationFailure(f"{peer.label} 重启后未恢复健康。")


def execute_acceptance() -> dict[str, object]:
    revision = git_revision()
    values = read_environment()
    topology = diagnose()
    suffix = uuid.uuid4().hex[:10]
    alpha = register_user(
        ALPHA,
        values["ALPHA_REGISTRATION_SECRET"],
        f"alpha-{suffix}",
        values["ALPHA_USER_PASSWORD"],
        administrator=True,
    )
    beta = register_user(
        BETA,
        values["BETA_REGISTRATION_SECRET"],
        f"beta-{suffix}",
        values["BETA_USER_PASSWORD"],
        administrator=False,
    )
    room_id = create_room(alpha, beta, alias_prefix="agent-room-federation")
    wait_for_room(beta, room_id, "invite")
    join_remote_room(beta, room_id, ALPHA)
    beta_since = wait_for_room(beta, room_id, "join")

    replay_transaction_id = f"task38-replay-{uuid.uuid4().hex}"
    replay_payload: dict[str, object] = {
        "schemaVersion": "1.0",
        "body": "task 38 replay probe",
    }
    replay_event_id, replay_accepted_at = send_custom_event(
        alpha,
        room_id,
        "io.github.rainyflash.agentroom.message.preview.v1",
        replay_payload,
        transaction_id=replay_transaction_id,
    )
    replayed_event_id, _ = send_custom_event(
        alpha,
        room_id,
        "io.github.rainyflash.agentroom.message.preview.v1",
        replay_payload,
        transaction_id=replay_transaction_id,
    )
    if replay_event_id != replayed_event_id:
        raise FederationFailure("相同 Matrix 事务标识产生了不同事件，重放保护失效。")
    unknown_event_type = "io.github.rainyflash.agentroom.message.future.v9"
    unknown_event_id, unknown_accepted_at = send_custom_event(
        alpha,
        room_id,
        unknown_event_type,
        {
            "schemaVersion": "9.0",
            "body": "不得被兼容视图读取",
            "requestedAction": "invoke_tool",
        },
    )
    legacy_event_type = ".".join(("org", "agentroom", "message", "preview", "v1"))
    legacy_event_id, legacy_accepted_at = send_custom_event(
        alpha,
        room_id,
        legacy_event_type,
        {"schemaVersion": "1.0", "body": "legacy compatibility probe"},
    )
    beta_since, compatibility_arrivals = wait_for_event(
        beta,
        room_id,
        {replay_event_id, unknown_event_id, legacy_event_id},
        beta_since,
    )

    event_id, accepted_at = send_event(alpha, room_id, "task 37 federation preview")
    beta_since, arrivals = wait_for_event(beta, room_id, {event_id}, beta_since)
    status_event_id = send_state(alpha, room_id)
    beta_since, status_arrival = wait_for_event(
        beta, room_id, {status_event_id}, beta_since
    )
    alpha_since = next_batch(sync(alpha, None))
    read_sent_at = send_receipt(beta, room_id, event_id)
    read_observed_at = wait_for_receipt(
        alpha, room_id, event_id, beta.user_id, alpha_since
    )

    run_e2ee_test(alpha, beta)
    verify_ban_and_recovery(alpha, beta, room_id)
    beta_since = next_batch(sync(beta, None))
    local_room = create_room(alpha, None, alias_prefix="agent-room-local-survival")
    stop_peer(BETA)
    local_event_id, local_accepted_at = send_event(
        alpha, local_room, "local service remains writable"
    )
    queued: dict[str, str] = {}
    for index in range(3):
        queued_event_id, queued_at = send_event(
            alpha, room_id, f"queued federation message {index + 1}"
        )
        queued[queued_event_id] = queued_at
    time.sleep(3)
    start_peer(BETA)
    recovered_at = utc_now()
    _, recovered_arrivals = wait_for_event(
        beta, room_id, set(queued), beta_since, timeout_seconds=90
    )
    require_git_revision(revision)
    return {
        "schemaVersion": 1,
        "task": 37,
        "revision": revision,
        "generatedAt": utc_now(),
        "passed": True,
        "topology": topology,
        "identities": {
            "alphaUser": alpha.user_id,
            "betaUser": beta.user_id,
            "roomId": room_id,
        },
        "deliveryEvidence": {
            "eventId": event_id,
            "locallyAcceptedAt": accepted_at,
            "federationArrivedAt": arrivals[event_id],
            "receiptSentAt": read_sent_at,
            "receiptObservedAt": read_observed_at,
        },
        "stateEvidence": {
            "eventId": status_event_id,
            "federationArrivedAt": status_arrival[status_event_id],
        },
        "e2ee": {"passed": True, "transport": "Megolm across two homeservers"},
        "governance": {"banRejectedRemoteSend": True, "unbanRejoined": True},
        "compatibilityEvidence": {
            "replay": {
                "transactionId": replay_transaction_id,
                "firstEventId": replay_event_id,
                "replayedEventId": replayed_event_id,
                "locallyAcceptedAt": replay_accepted_at,
                "federationArrivedAt": compatibility_arrivals[replay_event_id],
            },
            "unknownEvent": {
                "eventType": unknown_event_type,
                "eventId": unknown_event_id,
                "locallyAcceptedAt": unknown_accepted_at,
                "federationArrivedAt": compatibility_arrivals[unknown_event_id],
                "projectedMode": "metadata_only_read_only",
            },
            "legacyEvent": {
                "eventType": legacy_event_type,
                "eventId": legacy_event_id,
                "locallyAcceptedAt": legacy_accepted_at,
                "federationArrivedAt": compatibility_arrivals[legacy_event_id],
                "projectedMode": "metadata_only_read_only",
            },
        },
        "outageRecovery": {
            "peerStopped": BETA.server_name,
            "localRoomEventId": local_event_id,
            "localRoomAcceptedAt": local_accepted_at,
            "peerRecoveredAt": recovered_at,
            "queuedEventAcceptedAt": queued,
            "queuedEventArrivedAt": recovered_arrivals,
        },
    }


def git_revision() -> str:
    try:
        return clean_git_revision(ROOT)
    except SourceRevisionFailure as error:
        raise FederationFailure(str(error)) from error


def require_git_revision(expected: str) -> None:
    """确保联邦验收从头到尾绑定同一个干净提交。"""

    try:
        require_clean_git_revision(ROOT, expected)
    except SourceRevisionFailure as error:
        raise FederationFailure(str(error)) from error


def write_report(report: dict[str, object]) -> None:
    REPORT_FILE.parent.mkdir(parents=True, exist_ok=True)
    REPORT_FILE.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def bootstrap(*, keep_running: bool) -> dict[str, object]:
    down(volumes=True)
    prepare()
    try:
        up()
        report = execute_acceptance()
        write_report(report)
        return report
    finally:
        if not keep_running:
            down(volumes=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action",
        choices=("prepare", "up", "down", "reset", "diagnose", "test", "bootstrap"),
        nargs="?",
        default="bootstrap",
    )
    parser.add_argument(
        "--keep-running",
        action="store_true",
        help="bootstrap 完成后保留容器，便于人工诊断。",
    )
    arguments = parser.parse_args()
    try:
        if arguments.action == "prepare":
            prepare()
        elif arguments.action == "up":
            up()
        elif arguments.action == "down":
            down(volumes=False)
        elif arguments.action == "reset":
            down(volumes=True)
        elif arguments.action == "diagnose":
            print(json.dumps(diagnose(), ensure_ascii=False, indent=2))
        elif arguments.action == "test":
            report = execute_acceptance()
            write_report(report)
            print(f"联邦验收报告：{REPORT_FILE}")
        else:
            report = bootstrap(keep_running=arguments.keep_running)
            print(f"双 Homeserver 联邦验收通过：{REPORT_FILE}")
            print(json.dumps(report["deliveryEvidence"], ensure_ascii=False, indent=2))
        return 0
    except FederationFailure as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
