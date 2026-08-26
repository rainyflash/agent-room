#!/usr/bin/env python3
"""编排 Agent Room 内部纵向切片的真实服务、浏览器与本地进程。"""

from __future__ import annotations

import argparse
from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
import ctypes
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import subprocess
import sys
import threading
import time
from typing import Final, Protocol, TextIO
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
import uuid

if __package__:
    from .local_runtime import (
        LocalRuntimeError,
        bridge_runtime_environment,
        control_plane_runtime_environment,
        read_environment,
        required_value,
    )
    from .mcp_client import McpClientFailure, McpStdioClient
else:
    from local_runtime import (
        LocalRuntimeError,
        bridge_runtime_environment,
        control_plane_runtime_environment,
        read_environment,
        required_value,
    )
    from mcp_client import McpClientFailure, McpStdioClient


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"
COMPOSE_FILE: Final = ROOT / "infra" / "compose" / "compose.yaml"
MAIN_PROJECT_NAME: Final = "agent-room-dev"
VERTICAL_PROJECT_NAME: Final = "agent-room-vertical-24"
VERTICAL_ROOT: Final = ROOT / ".local" / "vertical"
BOOTSTRAP_RESULT: Final = VERTICAL_ROOT / "bootstrap.json"
CATALOG_RESULT: Final = VERTICAL_ROOT / "catalog.json"
LOG_ROOT: Final = ROOT / "artifacts" / "browser" / "task-24" / "services"
SECURITY_LOG_ROOT: Final = ROOT / "artifacts" / "browser" / "task-27" / "services"
CATALOG_SEED_ID: Final = "019d2c44-1dc5-7a5b-9e32-2f3c1d4b5a61"
CATALOG_SLUG: Final = "vertical-codex-lobby"
SENDER_SECURE_STORAGE_SERVICE: Final = "dev.agent-room.bridge.vertical-24.sender"
TARGET_SECURE_STORAGE_SERVICE: Final = "dev.agent-room.bridge.vertical-24.target"
SECURE_STORAGE_SERVICES: Final = (
    SENDER_SECURE_STORAGE_SERVICE,
    TARGET_SECURE_STORAGE_SERVICE,
)
SENDER_BRIDGE_DATA_ROOT: Final = VERTICAL_ROOT / "bridge-sender"
TARGET_BRIDGE_DATA_ROOT: Final = VERTICAL_ROOT / "bridge-target"
BRIDGE_DATA_ROOTS: Final = (SENDER_BRIDGE_DATA_ROOT, TARGET_BRIDGE_DATA_ROOT)
SECURE_STORAGE_ACCOUNTS: Final = (
    "device-signing-seed",
    "agent-instance-signing-seed-v1",
    "device-session-v1",
    "agent-runtime-session-v1",
    "matrix-store-passphrase-v1",
    "handoff-storage-key-v1",
    "bridge-ipc-installation-id-v1",
    "bridge-ipc-shared-secret-v1",
)
EXPECTED_MCP_TOOLS: Final = frozenset(
    {
        "agent_room_get_self",
        "agent_room_list_previews",
        "agent_room_get_presence",
        "agent_room_open_content",
        "agent_room_publish_status",
        "agent_room_send_message",
        "agent_room_consume_handoff",
        "agent_room_decline_handoff",
    }
)
SENSITIVE_NAME: Final = re.compile(r"(?:PASSWORD|SECRET|TOKEN|ACCESS_KEY)", re.IGNORECASE)
JWT_VALUE: Final = re.compile(
    r"\beyJ[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\b"
)
DEVICE_CODE_QUERY: Final = re.compile(r"(\buser_code=)[^&\s]+", re.IGNORECASE)
DEVICE_CODE_LINE: Final = re.compile(r"^(设备验证码：)(\S+)\s*$", re.MULTILINE)
UNREDACTED_DEVICE_CODE_QUERY: Final = re.compile(
    r"\buser_code=(?!\[已脱敏\])[^&\s]+", re.IGNORECASE
)
UNREDACTED_DEVICE_CODE_LINE: Final = re.compile(
    r"^设备验证码：(?!\[已脱敏\])\S+", re.MULTILINE
)
UUID_V7_TEXT: Final = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
    re.IGNORECASE,
)


class VerticalFailure(RuntimeError):
    """表示可诊断、不会泄露敏感值的纵向验收失败。"""


def configure_console_encoding() -> None:
    """统一纵向验收输出编码，避免 Windows 本地代码页杀死日志线程。"""
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if callable(reconfigure):
            reconfigure(encoding="utf-8", errors="replace")


class LogRedactor:
    """在日志落盘前移除本地凭据、JWT 与一次性设备码。"""

    def __init__(self, environment: Mapping[str, str]) -> None:
        self._known_secrets = tuple(
            sorted(
                {
                    value
                    for name, value in environment.items()
                    if SENSITIVE_NAME.search(name) and len(value) >= 8
                },
                key=len,
                reverse=True,
            )
        )

    def redact(self, line: str) -> str:
        sanitized = line
        for secret in self._known_secrets:
            sanitized = sanitized.replace(secret, "[已脱敏]")
        sanitized = JWT_VALUE.sub("[JWT 已脱敏]", sanitized)
        sanitized = DEVICE_CODE_QUERY.sub(r"\1[已脱敏]", sanitized)
        return DEVICE_CODE_LINE.sub(r"\1[已脱敏]", sanitized)

    @property
    def known_secrets(self) -> tuple[str, ...]:
        """只供落盘后的反向扫描使用，不把敏感值写入诊断信息。"""
        return self._known_secrets


LineObserver = Callable[[str], None]


class ProcessHealth(Protocol):
    def ensure_running(self) -> None: ...


class BridgeRuntimeObservation:
    """只在内存中捕获一次性设备码，并暴露可等待的运行时里程碑。"""

    def __init__(self) -> None:
        self._device_code: str | None = None
        self._device_code_ready = threading.Event()
        self._lock = threading.Lock()
        self._milestone_changed = threading.Condition(self._lock)
        self._agent_online_generation = 0

    def observe(self, line: str) -> None:
        match = DEVICE_CODE_LINE.fullmatch(line.strip())
        if match is not None:
            with self._lock:
                self._device_code = match.group(2)
            self._device_code_ready.set()
        with self._milestone_changed:
            if "Agent 已进入公共大厅并开始同步。" in line:
                self._agent_online_generation += 1
                self._milestone_changed.notify_all()

    def wait_for_device_code(
        self, process: ProcessHealth, *, timeout_seconds: float
    ) -> str:
        self._wait(
            self._device_code_ready,
            process,
            timeout_seconds=timeout_seconds,
            label="Bridge 设备码",
        )
        with self._lock:
            if self._device_code is None:
                raise VerticalFailure("Bridge 设备码观察状态不一致。")
            return self._device_code

    def wait_for_agent_online(
        self,
        process: ProcessHealth,
        *,
        after_generation: int = 0,
        timeout_seconds: float,
    ) -> int:
        return self._wait_for_generation(
            lambda: self._agent_online_generation,
            process,
            after_generation=after_generation,
            timeout_seconds=timeout_seconds,
            label="Agent 自动入厅",
        )

    @property
    def agent_online_generation(self) -> int:
        with self._lock:
            return self._agent_online_generation

    @staticmethod
    def _wait(
        event: threading.Event,
        process: ProcessHealth,
        *,
        timeout_seconds: float,
        label: str,
    ) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            process.ensure_running()
            if event.wait(timeout=min(0.25, max(0.0, deadline - time.monotonic()))):
                return
        raise VerticalFailure(f"{label} 未在 {timeout_seconds:.0f} 秒内出现。")

    def _wait_for_generation(
        self,
        read_generation: Callable[[], int],
        process: ProcessHealth,
        *,
        after_generation: int,
        timeout_seconds: float,
        label: str,
    ) -> int:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            process.ensure_running()
            with self._milestone_changed:
                generation = read_generation()
                if generation > after_generation:
                    return generation
                self._milestone_changed.wait(
                    timeout=min(0.25, max(0.0, deadline - time.monotonic()))
                )
        raise VerticalFailure(f"{label} 未在 {timeout_seconds:.0f} 秒内出现。")


@dataclass(frozen=True)
class AuthorizedBridgeRuntime:
    """纵向验收中已授权 Bridge 的唯一运行时句柄。"""

    process: "ManagedProcess"
    environment: Mapping[str, str]
    observation: BridgeRuntimeObservation
    device_code: str


class ManagedProcess(AbstractContextManager["ManagedProcess"]):
    """拥有一个子进程树，并保证输出先脱敏再写入验收证据。"""

    def __init__(
        self,
        *,
        name: str,
        command: Sequence[str],
        environment: Mapping[str, str],
        log_path: Path,
        redactor: LogRedactor,
        on_line: LineObserver | None = None,
    ) -> None:
        self._name = name
        self._command = tuple(command)
        self._environment = dict(environment)
        self._log_path = log_path
        self._redactor = redactor
        self._on_line = on_line
        self._process: subprocess.Popen[str] | None = None
        self._reader: threading.Thread | None = None
        self._log: TextIO | None = None

    @property
    def process(self) -> subprocess.Popen[str]:
        if self._process is None:
            raise VerticalFailure(f"进程 {self._name} 尚未启动。")
        return self._process

    def start(self) -> "ManagedProcess":
        if self._process is not None:
            raise VerticalFailure(f"进程 {self._name} 不能重复启动。")
        self._log_path.parent.mkdir(parents=True, exist_ok=True)
        self._log = self._log_path.open("w", encoding="utf-8", newline="\n")
        creation_flags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
        self._process = subprocess.Popen(
            self._command,
            cwd=ROOT,
            env=self._environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            creationflags=creation_flags,
            start_new_session=os.name != "nt",
        )
        self._reader = threading.Thread(
            target=self._pump_output,
            name=f"agent-room-{self._name}-log",
            daemon=True,
        )
        self._reader.start()
        return self

    def ensure_running(self) -> None:
        return_code = self.process.poll()
        if return_code is not None:
            raise VerticalFailure(
                f"进程 {self._name} 提前退出，退出码 {return_code}；请检查 {self._log_path}。"
            )

    def stop(self) -> None:
        process = self._process
        if process is None:
            return
        if process.poll() is None:
            self._request_graceful_stop(process)
            try:
                process.wait(timeout=12)
            except subprocess.TimeoutExpired:
                self._force_stop_tree(process)
                process.wait(timeout=10)
        if self._reader is not None:
            self._reader.join(timeout=5)
        if self._log is not None:
            self._log.close()
        self._reader = None
        self._log = None
        self._process = None

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        self.stop()

    def _pump_output(self) -> None:
        process = self.process
        stream = process.stdout
        log = self._log
        if stream is None or log is None:
            return
        for line in stream:
            if self._on_line is not None:
                self._on_line(line)
            sanitized = self._redactor.redact(line)
            log.write(sanitized)
            log.flush()
            print(f"[{self._name}] {sanitized}", end="", flush=True)

    @staticmethod
    def _request_graceful_stop(process: subprocess.Popen[str]) -> None:
        try:
            if os.name == "nt":
                process.send_signal(signal.CTRL_BREAK_EVENT)
            else:
                os.killpg(process.pid, signal.SIGTERM)
        except (OSError, ProcessLookupError):
            process.terminate()

    @staticmethod
    def _force_stop_tree(process: subprocess.Popen[str]) -> None:
        if os.name == "nt":
            subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                capture_output=True,
                check=False,
                text=True,
            )
            return
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return


class ProcessStack(AbstractContextManager["ProcessStack"]):
    def __init__(self) -> None:
        self._processes: list[ManagedProcess] = []

    def start(self, process: ManagedProcess) -> ManagedProcess:
        started = process.start()
        self._processes.append(started)
        return started

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        for process in reversed(self._processes):
            process.stop()


class IsolatedInfrastructure(AbstractContextManager["IsolatedInfrastructure"]):
    """临时占用本地端口运行全新依赖，并在退出时恢复原开发环境。"""

    def __init__(self) -> None:
        self._main_services: tuple[str, ...] = ()
        self._entered = False

    def __enter__(self) -> "IsolatedInfrastructure":
        if self._entered:
            raise VerticalFailure("隔离基础设施不能重复进入。")
        self._entered = True
        self._main_services = running_compose_services(MAIN_PROJECT_NAME)
        try:
            remove_vertical_infrastructure()
            if self._main_services:
                run_checked(
                    [
                        *compose_command(MAIN_PROJECT_NAME),
                        "stop",
                        *self._main_services,
                    ]
                )
            run_checked(
                [
                    *compose_command(VERTICAL_PROJECT_NAME),
                    "up",
                    "--detach",
                    "--wait",
                    "--wait-timeout",
                    "420",
                ]
            )
        except BaseException:
            self._cleanup(original_failure=True)
            raise
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        self._cleanup(original_failure=exc_value is not None)

    def _cleanup(self, *, original_failure: bool) -> None:
        failures: list[str] = []
        try:
            remove_vertical_infrastructure()
        except (OSError, VerticalFailure, subprocess.SubprocessError) as error:
            failures.append(f"隔离环境清理失败：{error}")
        if self._main_services:
            try:
                run_checked(
                    [
                        *compose_command(MAIN_PROJECT_NAME),
                        "start",
                        *self._main_services,
                    ]
                )
            except (OSError, VerticalFailure, subprocess.SubprocessError) as error:
                failures.append(f"原开发环境恢复失败：{error}")
        self._main_services = ()
        if not failures:
            return
        message = "；".join(failures)
        if original_failure:
            print(message, file=sys.stderr)
            return
        raise VerticalFailure(message)


class IsolatedServiceInterruption(
    AbstractContextManager["IsolatedServiceInterruption"]
):
    """只允许中断纵向环境中显式登记的依赖，并保证退出时恢复。"""

    _ALLOWED_SERVICES: Final = frozenset({"synapse"})

    def __init__(self, service: str) -> None:
        if service not in self._ALLOWED_SERVICES:
            raise VerticalFailure(f"拒绝中断未经审计的纵向服务：{service}。")
        self._service = service
        self._interrupted = False

    def __enter__(self) -> "IsolatedServiceInterruption":
        run_checked(
            [*compose_command(VERTICAL_PROJECT_NAME), "stop", self._service]
        )
        self._interrupted = True
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        if not self._interrupted:
            return
        try:
            run_checked(
                [*compose_command(VERTICAL_PROJECT_NAME), "start", self._service]
            )
        except (OSError, VerticalFailure, subprocess.SubprocessError) as error:
            if exc_value is not None:
                print(f"纵向服务 {self._service} 恢复失败：{error}", file=sys.stderr)
                return
            raise
        finally:
            self._interrupted = False


class IsolatedBridgeState(AbstractContextManager["IsolatedBridgeState"]):
    """只清理纵向验收专属目录和凭据命名空间。"""

    def __enter__(self) -> "IsolatedBridgeState":
        reset_bridge_data_roots()
        clear_vertical_secure_storage()
        for data_root in BRIDGE_DATA_ROOTS:
            data_root.mkdir(parents=True, exist_ok=True)
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        failures: list[str] = []
        try:
            reset_bridge_data_roots()
        except (OSError, VerticalFailure) as error:
            failures.append(f"Bridge 测试目录清理失败：{error}")
        try:
            clear_vertical_secure_storage()
        except (OSError, VerticalFailure, subprocess.SubprocessError) as error:
            failures.append(f"Bridge 测试凭据清理失败：{error}")
        if not failures:
            return
        message = "；".join(failures)
        if exc_value is not None:
            print(message, file=sys.stderr)
            return
        raise VerticalFailure(message)


def executable(name: str) -> str:
    resolved = shutil.which(name)
    if resolved is None:
        raise VerticalFailure(f"未找到必需命令：{name}。")
    return resolved


def run_checked(
    command: Sequence[str],
    *,
    environment: Mapping[str, str] | None = None,
) -> None:
    completed = subprocess.run(
        tuple(command),
        cwd=ROOT,
        env=None if environment is None else dict(environment),
        check=False,
    )
    if completed.returncode != 0:
        raise VerticalFailure(f"命令失败，退出码 {completed.returncode}。")


def compose_command(project_name: str) -> list[str]:
    if project_name not in {MAIN_PROJECT_NAME, VERTICAL_PROJECT_NAME}:
        raise VerticalFailure(f"拒绝操作未经审计的 Compose 项目：{project_name}。")
    return [
        executable("docker"),
        "compose",
        "--progress",
        "plain",
        "--project-name",
        project_name,
        "--env-file",
        str(ENV_FILE),
        "--file",
        str(COMPOSE_FILE),
    ]


def running_compose_services(project_name: str) -> tuple[str, ...]:
    completed = subprocess.run(
        [
            *compose_command(project_name),
            "ps",
            "--services",
            "--filter",
            "status=running",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    if completed.returncode != 0:
        raise VerticalFailure(f"无法读取 Compose 项目状态：{project_name}。")
    return tuple(line.strip() for line in completed.stdout.splitlines() if line.strip())


def remove_vertical_infrastructure() -> None:
    if VERTICAL_PROJECT_NAME != "agent-room-vertical-24":
        raise VerticalFailure("拒绝清理未知纵向验收项目。")
    run_checked(
        [
            *compose_command(VERTICAL_PROJECT_NAME),
            "down",
            "--volumes",
            "--remove-orphans",
        ]
    )


def reset_bridge_data_roots() -> None:
    expected_parent = VERTICAL_ROOT.resolve()
    for data_root in BRIDGE_DATA_ROOTS:
        target = data_root.resolve()
        if target.parent != expected_parent or target.name not in {
            "bridge-sender",
            "bridge-target",
        }:
            raise VerticalFailure("拒绝清理未经审计的 Bridge 测试目录。")
        if target.exists():
            shutil.rmtree(target)


def clear_vertical_secure_storage() -> None:
    if os.name == "nt":
        for service in SECURE_STORAGE_SERVICES:
            for account in SECURE_STORAGE_ACCOUNTS:
                delete_windows_credential(windows_credential_target(service, account))
        return
    if sys.platform == "darwin":
        for service in SECURE_STORAGE_SERVICES:
            for account in SECURE_STORAGE_ACCOUNTS:
                completed = subprocess.run(
                    [
                        executable("security"),
                        "delete-generic-password",
                        "-s",
                        service,
                        "-a",
                        account,
                    ],
                    cwd=ROOT,
                    capture_output=True,
                    check=False,
                )
                if completed.returncode not in {0, 44}:
                    raise VerticalFailure("无法清理 macOS 纵向验收凭据。")
        return
    secret_tool = shutil.which("secret-tool")
    if secret_tool is None:
        raise VerticalFailure("Linux 纵向验收需要 secret-tool 清理隔离凭据。")
    for service in SECURE_STORAGE_SERVICES:
        for account in SECURE_STORAGE_ACCOUNTS:
            completed = subprocess.run(
                [
                    secret_tool,
                    "clear",
                    "service",
                    service,
                    "username",
                    account,
                ],
                cwd=ROOT,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise VerticalFailure("无法清理 Linux 纵向验收凭据。")


def windows_credential_target(service: str, account: str) -> str:
    """匹配 windows-native-keyring-store 的默认 user.service 映射。"""
    return f"{account}.{service}"


def delete_windows_credential(target: str) -> None:
    credential_type_generic = 1
    error_not_found = 1168
    advapi32 = ctypes.WinDLL("Advapi32.dll", use_last_error=True)
    delete = advapi32.CredDeleteW
    delete.argtypes = (ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32)
    delete.restype = ctypes.c_int
    if delete(target, credential_type_generic, 0):
        return
    error_code = ctypes.get_last_error()
    if error_code != error_not_found:
        raise VerticalFailure(f"Windows 凭据清理失败，错误码 {error_code}。")


def prepare_environment() -> dict[str, str]:
    node = executable("node")
    run_checked(
        [node, "tools/run-powershell.mjs", "tools/dev-infra.ps1", "prepare"]
    )
    environment = read_environment()
    required_value(environment, "SEED_ADMIN_PASSWORD")
    return environment


def initialize_isolated_dependencies() -> None:
    process_environment = os.environ.copy()
    process_environment["AGENT_ROOM_COMPOSE_PROJECT_NAME"] = VERTICAL_PROJECT_NAME
    run_checked(
        [sys.executable, "tools/database.py", "migrate"],
        environment=process_environment,
    )
    run_checked(
        [
            executable("node"),
            "tools/run-powershell.mjs",
            "tools/dev-seed.ps1",
            "-EnvFile",
            str(ENV_FILE),
            "-ProjectName",
            VERTICAL_PROJECT_NAME,
        ]
    )


def compose_psql(sql: str) -> str:
    command = [
        executable("docker"),
        "compose",
        "--project-name",
        VERTICAL_PROJECT_NAME,
        "--env-file",
        str(ENV_FILE),
        "--file",
        str(COMPOSE_FILE),
        "exec",
        "-T",
        "postgres",
        "psql",
        "--set=ON_ERROR_STOP=1",
        "--quiet",
        "--tuples-only",
        "--no-align",
        "--username=agent_room_bootstrap",
        "--dbname=agent_room",
    ]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        input=sql,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    if completed.returncode != 0:
        raise VerticalFailure("纵向验收数据库夹具写入失败。")
    return completed.stdout.strip()


def seed_public_catalog() -> str:
    result = compose_psql(
        f"""
INSERT INTO agent_room.room_catalog_entry (
    id, kind, slug, name, description, language,
    visibility, status, created_at, updated_at
) VALUES (
    '{CATALOG_SEED_ID}', 'public_lobby', '{CATALOG_SLUG}',
    'Vertical Codex Lobby', 'Task 24 real vertical slice', 'en',
    'public', 'active', now(), now()
)
ON CONFLICT (slug) WHERE slug IS NOT NULL DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    language = EXCLUDED.language,
    visibility = EXCLUDED.visibility,
    status = EXCLUDED.status,
    updated_at = now()
RETURNING id::text;
"""
    )
    catalog_id = result.splitlines()[-1].strip() if result else ""
    require_uuid_v7(catalog_id, "公共大厅目录标识")
    write_json(CATALOG_RESULT, {"catalogId": catalog_id, "slug": CATALOG_SLUG})
    return catalog_id


def build_runtime_binaries(
    packages: Sequence[str] = (
        "agent-room-control-plane",
        "agent-room-bridge",
        "agent-room-codex-mcp",
    ),
) -> None:
    package_arguments = [argument for package in packages for argument in ("-p", package)]
    run_checked(
        [
            executable("cargo"),
            "build",
            "--locked",
            *package_arguments,
        ]
    )


def runtime_binary(name: str) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    path = ROOT / "target" / "debug" / f"{name}{suffix}"
    if not path.is_file():
        raise VerticalFailure(f"缺少已构建运行时：{path}。")
    return path


def wait_for_http(
    url: str,
    process: ManagedProcess,
    *,
    timeout_seconds: float,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        process.ensure_running()
        try:
            request = Request(url, headers={"Accept": "text/html,application/json"})
            with urlopen(request, timeout=2) as response:
                if http_status_is_ready(response.status):
                    return
        except (ConnectionError, TimeoutError, URLError):
            pass
        time.sleep(0.4)
    raise VerticalFailure(f"服务 {url} 未在 {timeout_seconds:.0f} 秒内就绪。")


def wait_for_local_https(
    url: str,
    process: ManagedProcess,
    *,
    timeout_seconds: float,
) -> None:
    """通过本机 Caddy 自签证书入口验证真实浏览器路由。"""
    curl = executable("curl")
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        process.ensure_running()
        completed = subprocess.run(
            [
                curl,
                "--fail",
                "--silent",
                "--show-error",
                "--insecure",
                "--max-time",
                "2",
                url,
            ],
            cwd=ROOT,
            capture_output=True,
            check=False,
        )
        if completed.returncode == 0:
            return
        time.sleep(0.4)
    raise VerticalFailure(f"本机 HTTPS 服务 {url} 未在 {timeout_seconds:.0f} 秒内就绪。")


def http_status_is_ready(status: int) -> bool:
    """只把成功响应视为就绪；重定向和客户端错误都必须继续等待。"""
    return 200 <= status < 300


def verify_error_correlation() -> str:
    """对真实控制面触发 404，并验证请求、响应头与错误体使用同一关联 ID。"""
    correlation_id = new_uuid_v7()
    request = Request(
        "http://127.0.0.1:8090/task-24/missing",
        headers={
            "Accept": "application/json",
            "x-correlation-id": correlation_id,
        },
    )
    try:
        with urlopen(request, timeout=5) as response:
            raise VerticalFailure(
                f"真实错误请求意外返回成功状态 {response.status}。"
            )
    except HTTPError as response:
        if response.code != 404:
            raise VerticalFailure(
                f"真实错误请求应返回 404，实际为 {response.code}。"
            ) from response
        response_correlation_id = response.headers.get("x-correlation-id")
        try:
            payload: object = json.loads(response.read().decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise VerticalFailure("真实错误响应不是有效 UTF-8 JSON。") from error

    body = require_object(payload, "真实错误响应")
    if response_correlation_id != correlation_id:
        raise VerticalFailure("真实错误响应头没有保留请求关联 ID。")
    if body.get("correlationId") != correlation_id:
        raise VerticalFailure("真实错误响应体没有保留请求关联 ID。")
    if body.get("code") != "http.route_not_found":
        raise VerticalFailure("真实错误响应没有返回稳定错误码。")
    return correlation_id


def start_control_plane(
    processes: ProcessStack,
    environment: Mapping[str, str],
    redactor: LogRedactor,
    log_root: Path = LOG_ROOT,
) -> ManagedProcess:
    control_plane = processes.start(
        ManagedProcess(
            name="control-plane",
            command=[str(runtime_binary("agent-room-control-plane"))],
            environment=vertical_control_plane_environment(environment),
            log_path=log_root / "control-plane.log",
            redactor=redactor,
        )
    )
    wait_for_http(
        "http://127.0.0.1:8090/health/live",
        control_plane,
        timeout_seconds=120,
    )
    return control_plane


def vertical_control_plane_environment(
    environment: Mapping[str, str],
) -> dict[str, str]:
    """仅为隔离纵向环境开放宿主监听，使 Docker Caddy 可访问控制平面。"""
    runtime_environment = control_plane_runtime_environment(
        environment, enable_telemetry=True
    )
    runtime_environment["AGENT_ROOM_BIND_ADDRESS"] = "0.0.0.0:8090"
    return runtime_environment


def web_preview_command() -> list[str]:
    """返回可被宿主机与 Docker 网关共同访问的发布预览命令。"""
    return [
        executable("node"),
        "apps/web/node_modules/vite/bin/vite.js",
        "preview",
        "apps/web",
        "--host",
        "0.0.0.0",
        "--port",
        "5173",
        "--strictPort",
    ]


def start_web(
    processes: ProcessStack,
    redactor: LogRedactor,
    log_root: Path = LOG_ROOT,
    environment_overrides: Mapping[str, str] | None = None,
) -> ManagedProcess:
    """构建并启动发布形态 Web，确保 CSP 验收不依赖开发服务器特权。"""
    web_environment = os.environ.copy()
    web_environment.update(environment_overrides or {})
    run_checked(
        [
            executable("node"),
            "apps/web/node_modules/vite/bin/vite.js",
            "build",
            "apps/web",
            "--mode",
            "vertical",
        ],
        environment=web_environment,
    )
    web = processes.start(
        ManagedProcess(
            name="web",
            command=web_preview_command(),
            environment=web_environment,
            log_path=log_root / "web.log",
            redactor=redactor,
        )
    )
    wait_for_http("http://127.0.0.1:5173/connect", web, timeout_seconds=60)
    wait_for_local_https(
        "https://app.agent-room.localhost:18443/connect",
        web,
        timeout_seconds=60,
    )
    return web


def bootstrap_agent(environment: Mapping[str, str]) -> dict[str, str]:
    VERTICAL_ROOT.mkdir(parents=True, exist_ok=True)
    BOOTSTRAP_RESULT.unlink(missing_ok=True)
    process_environment = os.environ.copy()
    playwright_environment = process_environment.copy()
    playwright_environment.update(
        {
            "AGENT_ROOM_E2E_USERNAME": "developer",
            "AGENT_ROOM_E2E_PASSWORD": required_value(
                environment, "SEED_ADMIN_PASSWORD"
            ),
            "AGENT_ROOM_VERTICAL_BOOTSTRAP_RESULT": str(BOOTSTRAP_RESULT),
        }
    )
    run_checked(
        [
            executable("node"),
            "apps/web/node_modules/@playwright/test/cli.js",
            "test",
            "--config",
            "apps/web/playwright.vertical.config.ts",
            "bootstrap.e2e.ts",
        ],
        environment=playwright_environment,
    )

    result = read_string_object(BOOTSTRAP_RESULT)
    for name in ("agentId", "principalId"):
        require_uuid_v7(result.get(name, ""), name)
    for name in ("agentMatrixUserId", "userMatrixUserId"):
        if not result.get(name, "").startswith("@"):
            raise VerticalFailure(f"浏览器引导结果缺少有效 {name}。")
    return result


def verify_browser_security(environment: Mapping[str, str]) -> None:
    """在全新 Synapse 中运行三设备交叉签名、SAS 与恢复验收。"""
    playwright_environment = os.environ.copy()
    playwright_environment.update(
        {
            "AGENT_ROOM_E2E_USERNAME": "developer",
            "AGENT_ROOM_E2E_PASSWORD": required_value(
                environment, "SEED_ADMIN_PASSWORD"
            ),
            "AGENT_ROOM_VERTICAL_EVIDENCE_TASK": "task-27",
        }
    )
    run_checked(
        [
            executable("node"),
            "apps/web/node_modules/@playwright/test/cli.js",
            "test",
            "--config",
            "apps/web/playwright.vertical.config.ts",
            "security.e2e.ts",
        ],
        environment=playwright_environment,
    )


def start_authorized_bridge(
    *,
    processes: ProcessStack,
    environment: Mapping[str, str],
    catalog_id: str,
    agent_id: str,
    runtime_name: str,
    data_root: Path,
    secure_storage_service: str,
    redactor: LogRedactor,
) -> AuthorizedBridgeRuntime:
    if runtime_name not in {"bridge-sender", "bridge-target"}:
        raise VerticalFailure(f"拒绝启动未经审计的 Bridge 角色：{runtime_name}。")
    if data_root not in BRIDGE_DATA_ROOTS:
        raise VerticalFailure(f"拒绝使用未经审计的 Bridge 数据目录：{data_root}。")
    if secure_storage_service not in SECURE_STORAGE_SERVICES:
        raise VerticalFailure("拒绝使用未经审计的 Bridge 安全存储命名空间。")
    bridge_environment = bridge_runtime_environment(
        data_root=data_root.resolve(),
        agent_id=agent_id,
        public_lobby_catalog_id=catalog_id,
        secure_storage_service=secure_storage_service,
    )
    observation = BridgeRuntimeObservation()
    bridge = processes.start(
        ManagedProcess(
            name=runtime_name,
            command=[str(runtime_binary("agent-room-bridge"))],
            environment=bridge_environment,
            log_path=LOG_ROOT / f"{runtime_name}.log",
            redactor=redactor,
            on_line=observation.observe,
        )
    )
    device_code = observation.wait_for_device_code(bridge, timeout_seconds=90)
    approve_device_grant(environment, device_code)
    observation.wait_for_agent_online(bridge, timeout_seconds=180)
    return AuthorizedBridgeRuntime(
        process=bridge,
        environment=bridge_environment,
        observation=observation,
        device_code=device_code,
    )


def verify_matrix_disconnect_and_recovery(
    target: AuthorizedBridgeRuntime,
    peers: Sequence[AuthorizedBridgeRuntime],
    redactor: LogRedactor,
) -> int:
    """真实停止 Synapse，验证 MCP 暂时拒绝服务并恢复到新的在线代次。"""
    runtimes = (target, *peers)
    online_generations = tuple(
        runtime.observation.agent_online_generation for runtime in runtimes
    )
    with IsolatedServiceInterruption("synapse"):
        wait_for_mcp_runtime_unavailable(
            bridge_environment=target.environment,
            bridge_process=target.process,
            redactor=redactor,
            timeout_seconds=45,
        )
    wait_for_http(
        "http://127.0.0.1:18008/_matrix/client/versions",
        target.process,
        timeout_seconds=120,
    )
    recovered_generations = tuple(
        runtime.observation.wait_for_agent_online(
            runtime.process,
            after_generation=online_generation,
            timeout_seconds=180,
        )
        for runtime, online_generation in zip(
            runtimes, online_generations, strict=True
        )
    )
    return recovered_generations[0]


def wait_for_mcp_runtime_unavailable(
    *,
    bridge_environment: Mapping[str, str],
    bridge_process: ManagedProcess,
    redactor: LogRedactor,
    timeout_seconds: float,
) -> None:
    """通过真实插件边界观察断网态，禁止把日志字符串当作状态协议。"""
    deadline = time.monotonic() + timeout_seconds
    with McpStdioClient(
        command=[str(runtime_binary("agent-room-codex-mcp"))],
        working_directory=ROOT,
        environment=bridge_environment,
        stderr_path=LOG_ROOT / "codex-mcp-reconnect.log",
        sanitize_line=redactor.redact,
    ) as client:
        while time.monotonic() < deadline:
            bridge_process.ensure_running()
            result = client.call_tool_result("agent_room_get_self", {})
            structured = result.get("structuredContent")
            if result.get("isError") is True and isinstance(structured, dict):
                if structured.get("code") == "bridge.agent_runtime_unavailable":
                    return
            time.sleep(0.4)
    raise VerticalFailure(
        f"Synapse 中断后 MCP 未在 {timeout_seconds:.0f} 秒内进入可恢复拒绝态。"
    )


def approve_device_grant(environment: Mapping[str, str], device_code: str) -> None:
    approval_environment = os.environ.copy()
    approval_environment.update(
        {
            "AGENT_ROOM_TEST_DEVICE_USER_CODE": device_code,
            "AGENT_ROOM_TEST_OIDC_USERNAME": "developer",
            "AGENT_ROOM_TEST_OIDC_PASSWORD": required_value(
                environment, "SEED_ADMIN_PASSWORD"
            ),
        }
    )
    run_checked(
        [
            executable("node"),
            "tools/run-powershell.mjs",
            "tools/device-grant-approve.ps1",
            "-BaseUrl",
            "http://127.0.0.1:18080",
        ],
        environment=approval_environment,
    )


def verify_mcp_workflow(
    *,
    target_bridge_environment: Mapping[str, str],
    sender_bridge_environment: Mapping[str, str],
    expected_agent_id: str,
    principal_id: str,
    redactor: LogRedactor,
) -> dict[str, str]:
    room = active_room_for_agent(expected_agent_id)
    with (
        McpStdioClient(
            command=[str(runtime_binary("agent-room-codex-mcp"))],
            working_directory=ROOT,
            environment=target_bridge_environment,
            stderr_path=LOG_ROOT / "codex-mcp.log",
            sanitize_line=redactor.redact,
        ) as client,
        McpStdioClient(
            command=[str(runtime_binary("agent-room-codex-mcp"))],
            working_directory=ROOT,
            environment=sender_bridge_environment,
            stderr_path=LOG_ROOT / "codex-mcp-sender.log",
            sanitize_line=redactor.redact,
        ) as sender_client,
    ):
        tools = frozenset(client.list_tool_names())
        if tools != EXPECTED_MCP_TOOLS:
            missing = sorted(EXPECTED_MCP_TOOLS - tools)
            unexpected = sorted(tools - EXPECTED_MCP_TOOLS)
            raise VerticalFailure(
                f"MCP 工具面不匹配；缺少 {missing}，多出 {unexpected}。"
            )
        identity_response = client.call_tool("agent_room_get_self", {})
        identity = verify_mcp_identity_response(identity_response, expected_agent_id)
        sender_identity = verify_mcp_identity_response(
            sender_client.call_tool("agent_room_get_self", {}), expected_agent_id
        )
        if sender_identity["agentInstanceId"] == identity["agentInstanceId"]:
            raise VerticalFailure("交接发送端与 Codex 接收端错误地复用了同一实例。")
        verify_mcp_status_publication(client, room["matrixRoomId"])
        wait_for_mcp_presence(
            client,
            room_id=room["matrixRoomId"],
            expected_agent_id=expected_agent_id,
            expected_instance_id=identity["agentInstanceId"],
            timeout_seconds=45,
        )
        message = send_mcp_vertical_message(sender_client, room["matrixRoomId"])
        preview = wait_for_mcp_preview(
            client,
            room_id=room["matrixRoomId"],
            submission=message,
            timeout_seconds=45,
        )
        verify_mcp_opened_content(client, room["matrixRoomId"], preview, message)
        handoff_id = approve_real_handoff(
            bridge_environment=sender_bridge_environment,
            principal_id=principal_id,
            room_id=room["matrixRoomId"],
            source_content_id=require_text(
                require_object(preview.get("content"), "MCP 正文引用").get(
                    "contentId"
                ),
                "MCP 正文标识",
            ),
            target_agent_id=expected_agent_id,
            target_instance_id=identity["agentInstanceId"],
        )
        wait_for_mcp_handoff_consumption(
            client,
            handoff_id=handoff_id,
            room_id=room["matrixRoomId"],
            source_event_id=message["eventId"],
            expected_body=message["body"],
            timeout_seconds=45,
        )
        reply = send_mcp_vertical_reply(
            client,
            room_id=room["matrixRoomId"],
            reply_to_message_id=require_text(
                preview.get("messageId"), "MCP 源消息标识"
            ),
            handoff_id=handoff_id,
        )
        reply_preview = wait_for_mcp_preview(
            client,
            room_id=room["matrixRoomId"],
            submission=reply,
            timeout_seconds=45,
        )
        verify_mcp_opened_content(
            client, room["matrixRoomId"], reply_preview, reply
        )

    return {
        "agentInstanceId": identity["agentInstanceId"],
        "handoffId": handoff_id,
        "matrixDeviceId": identity["matrixDeviceId"],
        "senderAgentInstanceId": sender_identity["agentInstanceId"],
        "roomInstanceId": room["roomInstanceId"],
        "matrixRoomId": room["matrixRoomId"],
        "messageId": require_text(preview.get("messageId"), "MCP 消息标识"),
        "replyMessageId": require_text(
            reply_preview.get("messageId"), "MCP 回复消息标识"
        ),
        "contentId": require_text(
            require_object(preview.get("content"), "MCP 正文引用").get("contentId"),
            "MCP 正文标识",
        ),
    }


def verify_mcp_identity_response(
    response: Mapping[str, object], expected_agent_id: str
) -> dict[str, str]:
    if response.get("type") != "self_summary":
        raise VerticalFailure("MCP 当前身份返回了错误响应类型。")
    summary = require_object(response.get("summary"), "MCP 当前身份摘要")
    agent = require_object(summary.get("agent"), "MCP Agent 身份")
    if agent.get("agentId") != expected_agent_id:
        raise VerticalFailure("MCP Agent 身份与浏览器创建结果不一致。")
    instance_id = require_text(summary.get("instanceId"), "MCP Agent 实例标识")
    require_uuid_v7(instance_id, "MCP Agent 实例标识")
    return {
        "agentInstanceId": instance_id,
        "matrixDeviceId": require_text(
            summary.get("matrixDeviceId"), "MCP Matrix 设备标识"
        ),
    }


def verify_mcp_status_publication(client: McpStdioClient, room_id: str) -> None:
    response = client.call_tool(
        "agent_room_publish_status",
        {
            "roomId": room_id,
            "status": "working",
            "taskSummary": "Task 24 vertical verification",
            "progressBasisPoints": 5_000,
        },
    )
    if response.get("type") != "published_status":
        raise VerticalFailure("MCP 状态发布返回了错误响应类型。")
    publication = require_object(response.get("publication"), "MCP 状态发布结果")
    if publication.get("roomId") != room_id or publication.get("status") != "working":
        raise VerticalFailure("MCP 状态发布结果与请求不一致。")
    lease_expiry = publication.get("leaseExpiresAtUnixMs")
    if not isinstance(lease_expiry, int) or lease_expiry <= int(time.time() * 1_000):
        raise VerticalFailure("MCP 状态发布没有返回有效租约。")


def wait_for_mcp_presence(
    client: McpStdioClient,
    *,
    room_id: str,
    expected_agent_id: str,
    expected_instance_id: str,
    timeout_seconds: float,
) -> dict[str, object]:
    """等待发布事件经过 Matrix 同步、验签和租约投影后重新可读。"""
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        response = client.call_tool(
            "agent_room_get_presence",
            {"roomId": room_id, "agentIds": [expected_agent_id]},
        )
        if response.get("type") != "presence":
            raise VerticalFailure("MCP Presence 返回了错误响应类型。")
        entries = response.get("entries")
        if not isinstance(entries, list):
            raise VerticalFailure("MCP Presence 缺少状态数组。")
        for item in entries:
            entry = require_object(item, "MCP Presence 状态")
            agent = require_object(entry.get("agent"), "MCP Presence Agent 身份")
            if (
                agent.get("agentId") != expected_agent_id
                or entry.get("instanceId") != expected_instance_id
            ):
                continue
            if entry.get("roomId") != room_id:
                raise VerticalFailure("MCP Presence 返回了错误房间的状态。")
            lease_expiry = entry.get("leaseExpiresAtUnixMs")
            observed_at = entry.get("observedAtUnixMs")
            if not isinstance(observed_at, int):
                raise VerticalFailure("MCP Presence 缺少有效观察时间。")
            if (
                entry.get("status") == "working"
                and isinstance(lease_expiry, int)
                and lease_expiry > int(time.time() * 1_000)
            ):
                return entry
        time.sleep(0.4)
    raise VerticalFailure(
        f"已发布状态未在 {timeout_seconds:.0f} 秒内经过 Matrix 验签投影重新可读。"
    )


def send_mcp_vertical_message(
    client: McpStdioClient, room_id: str
) -> dict[str, str]:
    submission_id = new_uuid_v7()
    title = f"Task 24 reply {submission_id[-8:]}"
    body = f"Real Agent Room vertical message. Submission: `{submission_id}`."
    response = client.call_tool(
        "agent_room_send_message",
        {
            "submissionId": submission_id,
            "roomId": room_id,
            "title": title,
            "summary": "Bridge and Codex MCP completed a real message round trip.",
            "body": body,
            "mediaType": "text/markdown",
            "language": "en",
            "sensitivity": "normal",
            "riskFlags": [],
            "provenance": "human_confirmed_agent",
            "replyToMessageId": None,
        },
    )
    if response.get("type") != "sent_message":
        raise VerticalFailure("MCP 消息发送返回了错误响应类型。")
    message = require_object(response.get("message"), "MCP 消息发送结果")
    if message.get("submissionId") != submission_id:
        raise VerticalFailure("MCP 消息发送结果的幂等标识不一致。")
    if message.get("state") != "submitted":
        raise VerticalFailure(f"MCP 消息没有确定提交，状态为 {message.get('state')}。")
    event_id = require_text(message.get("eventId"), "MCP Matrix 消息事件标识")
    if not event_id.startswith("$"):
        raise VerticalFailure("MCP Matrix 消息事件标识格式无效。")
    return {
        "submissionId": submission_id,
        "eventId": event_id,
        "title": title,
        "body": body,
    }


def send_mcp_vertical_reply(
    client: McpStdioClient,
    *,
    room_id: str,
    reply_to_message_id: str,
    handoff_id: str,
) -> dict[str, str]:
    submission_id = new_uuid_v7()
    title = f"Task 24 handoff reply {handoff_id[-8:]}"
    body = (
        "Real Codex plugin reply after one-time handoff consumption. "
        f"Handoff: `{handoff_id}`."
    )
    response = client.call_tool(
        "agent_room_send_message",
        {
            "submissionId": submission_id,
            "roomId": room_id,
            "title": title,
            "summary": "Codex consumed verified context and replied through the Bridge.",
            "body": body,
            "mediaType": "text/markdown",
            "language": "en",
            "sensitivity": "normal",
            "riskFlags": [],
            "provenance": "human_confirmed_agent",
            "replyToMessageId": reply_to_message_id,
        },
    )
    if response.get("type") != "sent_message":
        raise VerticalFailure("MCP 回复发送返回了错误响应类型。")
    reply = require_object(response.get("message"), "MCP 回复发送结果")
    if reply.get("submissionId") != submission_id or reply.get("state") != "submitted":
        raise VerticalFailure("MCP 回复没有确定提交。")
    event_id = require_text(reply.get("eventId"), "MCP 回复 Matrix 事件标识")
    if not event_id.startswith("$"):
        raise VerticalFailure("MCP 回复 Matrix 事件标识格式无效。")
    return {
        "submissionId": submission_id,
        "eventId": event_id,
        "title": title,
        "body": body,
    }


def wait_for_mcp_preview(
    client: McpStdioClient,
    *,
    room_id: str,
    submission: Mapping[str, str],
    timeout_seconds: float,
) -> dict[str, object]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        response = client.call_tool(
            "agent_room_list_previews",
            {"roomId": room_id, "beforeEventId": None, "limit": 20},
        )
        if response.get("type") != "message_previews":
            raise VerticalFailure("MCP 消息预览返回了错误响应类型。")
        previews = response.get("previews")
        if not isinstance(previews, list):
            raise VerticalFailure("MCP 消息预览缺少数组。")
        for item in previews:
            preview = require_object(item, "MCP 消息预览")
            if preview.get("eventId") == submission["eventId"]:
                if preview.get("title") != submission["title"]:
                    raise VerticalFailure("MCP 消息预览标题与已发送消息不一致。")
                return preview
        time.sleep(0.4)
    raise VerticalFailure(
        f"已发送消息未在 {timeout_seconds:.0f} 秒内进入 MCP 预览投影。"
    )


def verify_mcp_opened_content(
    client: McpStdioClient,
    room_id: str,
    preview: Mapping[str, object],
    submission: Mapping[str, str],
) -> None:
    content_reference = require_object(preview.get("content"), "MCP 正文引用")
    content_id = require_text(content_reference.get("contentId"), "MCP 正文标识")
    require_uuid_v7(content_id, "MCP 正文标识")
    response = client.call_tool(
        "agent_room_open_content",
        {"contentId": content_id},
    )
    if response.get("type") != "opened_content":
        raise VerticalFailure("MCP 正文读取返回了错误响应类型。")
    content = require_object(response.get("content"), "MCP 已打开正文")
    if content.get("sourceRoomId") != room_id:
        raise VerticalFailure("MCP 正文来源房间与预览不一致。")
    if content.get("sourceEventId") != submission["eventId"]:
        raise VerticalFailure("MCP 正文来源事件与预览不一致。")
    if content.get("body") != submission["body"]:
        raise VerticalFailure("MCP 正文未保持发送时的完整字节内容。")


def approve_real_handoff(
    *,
    bridge_environment: Mapping[str, str],
    principal_id: str,
    room_id: str,
    source_content_id: str,
    target_agent_id: str,
    target_instance_id: str,
) -> str:
    """通过桌面壳本机 IPC 适配器批准一次性交接，不给 MCP 越权批准能力。"""
    handoff_id = new_uuid_v7()
    environment = dict(bridge_environment)
    environment.update(
        {
            "AGENT_ROOM_TEST_HANDOFF_ID": handoff_id,
            "AGENT_ROOM_TEST_PRINCIPAL_ID": principal_id,
            "AGENT_ROOM_TEST_ROOM_ID": room_id,
            "AGENT_ROOM_TEST_SOURCE_CONTENT_ID": source_content_id,
            "AGENT_ROOM_TEST_TARGET_AGENT_ID": target_agent_id,
            "AGENT_ROOM_TEST_TARGET_INSTANCE_ID": target_instance_id,
        }
    )
    run_checked(
        [
            executable("cargo"),
            "test",
            "--locked",
            "-p",
            "agent-room-bridge-local-adapter",
            "--test",
            "desktop_handoff_real",
            "--",
            "--ignored",
            "--exact",
            "桌面壳可批准真实一次性交接",
        ],
        environment=environment,
    )
    return handoff_id


def wait_for_mcp_handoff_consumption(
    client: McpStdioClient,
    *,
    handoff_id: str,
    room_id: str,
    source_event_id: str,
    expected_body: str,
    timeout_seconds: float,
) -> None:
    """等待加密 To-Device 交付落库，再通过 Codex 工具原子消费并验证一次性。"""
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        result = client.call_tool_result(
            "agent_room_consume_handoff", {"handoffId": handoff_id}
        )
        structured = result.get("structuredContent")
        if not isinstance(structured, dict):
            raise VerticalFailure("MCP 交接消费缺少结构化响应。")
        if result.get("isError") is True:
            if structured.get("code") != "bridge.handoff_not_found":
                raise VerticalFailure(
                    f"MCP 交接消费返回非预期错误码 {structured.get('code')}。"
                )
            time.sleep(0.4)
            continue
        if structured.get("type") != "consumed_handoff":
            raise VerticalFailure("MCP 交接消费返回了错误响应类型。")
        handoff = require_object(structured.get("handoff"), "MCP 已消费交接")
        if handoff.get("handoffId") != handoff_id:
            raise VerticalFailure("MCP 消费了错误的一次性交接。")
        if handoff.get("sourceRoomId") != room_id:
            raise VerticalFailure("MCP 交接来源房间与已打开正文不一致。")
        if handoff.get("sourceEventId") != source_event_id:
            raise VerticalFailure("MCP 交接来源事件与已打开正文不一致。")
        if handoff.get("body") != expected_body:
            raise VerticalFailure("MCP 交接正文与已验证正文不一致。")
        verify_handoff_is_one_time(client, handoff_id)
        return
    raise VerticalFailure(f"一次性交接未在 {timeout_seconds:.0f} 秒内到达 Codex。")


def verify_handoff_is_one_time(client: McpStdioClient, handoff_id: str) -> None:
    second = client.call_tool_result(
        "agent_room_consume_handoff", {"handoffId": handoff_id}
    )
    structured = second.get("structuredContent")
    if (
        second.get("isError") is not True
        or not isinstance(structured, dict)
        or structured.get("code") != "bridge.handoff_already_resolved"
    ):
        raise VerticalFailure("一次性交接被重复消费，原子删除门禁失败。")


def active_room_for_agent(agent_id: str) -> dict[str, str]:
    result = compose_psql(
        f"""
SELECT room.id::text || '|' || room.matrix_room_id
FROM agent_room.room_capacity_reservation AS reservation
JOIN agent_room.room_instance AS room
  ON room.id = reservation.room_instance_id
WHERE reservation.agent_id = '{agent_id}'
  AND reservation.state = 'committed'
  AND room.state = 'active'
ORDER BY reservation.finalized_at DESC, reservation.id DESC
LIMIT 1;
"""
    )
    room_instance_id, separator, matrix_room_id = result.partition("|")
    if not separator or not matrix_room_id.startswith("!"):
        raise VerticalFailure("Bridge 自动入厅后没有形成有效房间归属。")
    require_uuid_v7(room_instance_id, "公共大厅分片标识")
    return {
        "roomInstanceId": room_instance_id,
        "matrixRoomId": matrix_room_id,
    }


def require_object(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(name, str) for name in value):
        raise VerticalFailure(f"{label} 必须是对象。")
    return {str(name): item for name, item in value.items()}


def require_text(value: object, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise VerticalFailure(f"{label} 必须是非空文本。")
    return value


def read_string_object(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise VerticalFailure(f"验收步骤没有生成结果文件：{path}。")
    try:
        payload: object = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise VerticalFailure(f"验收结果不是有效 JSON：{path}。") from error
    if not isinstance(payload, dict) or not all(
        isinstance(name, str) and isinstance(value, str)
        for name, value in payload.items()
    ):
        raise VerticalFailure(f"验收结果必须是字符串对象：{path}。")
    return payload


def write_json(path: Path, payload: Mapping[str, str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(dict(payload), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def verify_sanitized_logs(
    paths: Sequence[Path],
    redactor: LogRedactor,
    *,
    additional_secrets: Sequence[str] = (),
) -> tuple[str, ...]:
    """反向扫描真实落盘日志；任何已知凭据、JWT 或设备码残留都会失败。"""
    forbidden_values = tuple(
        value
        for value in (*redactor.known_secrets, *additional_secrets)
        if len(value) >= 4
    )
    issues: list[str] = []
    scanned: list[str] = []
    for path in paths:
        if not path.is_file():
            raise VerticalFailure(f"敏感日志扫描缺少预期文件：{path}。")
        text = path.read_text(encoding="utf-8", errors="replace")
        scanned.append(path.name)
        if any(secret in text for secret in forbidden_values):
            issues.append(f"{path.name}:known-secret")
        if JWT_VALUE.search(text) is not None:
            issues.append(f"{path.name}:jwt")
        if UNREDACTED_DEVICE_CODE_QUERY.search(text) is not None:
            issues.append(f"{path.name}:device-code-query")
        if UNREDACTED_DEVICE_CODE_LINE.search(text) is not None:
            issues.append(f"{path.name}:device-code-line")
    if issues:
        raise VerticalFailure(
            "敏感日志扫描发现未脱敏内容：" + ", ".join(sorted(set(issues)))
        )
    return tuple(scanned)


def require_uuid_v7(value: str, label: str) -> None:
    if not UUID_V7_TEXT.fullmatch(value):
        raise VerticalFailure(f"{label} 不是 UUIDv7。")
    try:
        parsed = uuid.UUID(value)
    except ValueError as error:
        raise VerticalFailure(f"{label} 不是有效 UUID。") from error
    if parsed.version != 7:
        raise VerticalFailure(f"{label} 不是 UUIDv7。")


def new_uuid_v7() -> str:
    """生成不依赖第三方包的 RFC 9562 UUIDv7。"""
    unix_milliseconds = int(time.time() * 1_000)
    if not 0 <= unix_milliseconds < (1 << 48):
        raise VerticalFailure("当前系统时间无法编码为 UUIDv7。")
    random_a = secrets.randbits(12)
    random_b = secrets.randbits(62)
    value = (
        (unix_milliseconds << 80)
        | (0b0111 << 76)
        | (random_a << 64)
        | (0b10 << 62)
        | random_b
    )
    return str(uuid.UUID(int=value))


def bootstrap() -> None:
    environment = prepare_environment()
    build_runtime_binaries()
    with IsolatedInfrastructure():
        initialize_isolated_dependencies()
        catalog_id = seed_public_catalog()
        redactor = LogRedactor(environment)
        with IsolatedBridgeState(), ProcessStack() as processes:
            start_control_plane(processes, environment, redactor)
            start_web(processes, redactor)
            correlation_id = verify_error_correlation()
            agent = bootstrap_agent(environment)
            sender_bridge = start_authorized_bridge(
                processes=processes,
                environment=environment,
                catalog_id=catalog_id,
                agent_id=agent["agentId"],
                runtime_name="bridge-sender",
                data_root=SENDER_BRIDGE_DATA_ROOT,
                secure_storage_service=SENDER_SECURE_STORAGE_SERVICE,
                redactor=redactor,
            )
            target_bridge = start_authorized_bridge(
                processes=processes,
                environment=environment,
                catalog_id=catalog_id,
                agent_id=agent["agentId"],
                runtime_name="bridge-target",
                data_root=TARGET_BRIDGE_DATA_ROOT,
                secure_storage_service=TARGET_SECURE_STORAGE_SERVICE,
                redactor=redactor,
            )
            recovery_generation = verify_matrix_disconnect_and_recovery(
                target_bridge, (sender_bridge,), redactor
            )
            runtime = verify_mcp_workflow(
                target_bridge_environment=target_bridge.environment,
                sender_bridge_environment=sender_bridge.environment,
                expected_agent_id=agent["agentId"],
                principal_id=agent["principalId"],
                redactor=redactor,
            )
        scanned_logs = verify_sanitized_logs(
            (
                LOG_ROOT / "control-plane.log",
                LOG_ROOT / "web.log",
                LOG_ROOT / "bridge-sender.log",
                LOG_ROOT / "bridge-target.log",
                LOG_ROOT / "codex-mcp-reconnect.log",
                LOG_ROOT / "codex-mcp-sender.log",
                LOG_ROOT / "codex-mcp.log",
            ),
            redactor,
            additional_secrets=(
                sender_bridge.device_code,
                target_bridge.device_code,
            ),
        )
    print(
        json.dumps(
            {
                "agentId": agent["agentId"],
                "agentInstanceId": runtime["agentInstanceId"],
                "catalogId": catalog_id,
                "contentId": runtime["contentId"],
                "correlationId": correlation_id,
                "handoffId": runtime["handoffId"],
                "logFilesScanned": str(len(scanned_logs)),
                "matrixRoomId": runtime["matrixRoomId"],
                "messageId": runtime["messageId"],
                "principalId": agent["principalId"],
                "recoveryGeneration": str(recovery_generation),
                "replyMessageId": runtime["replyMessageId"],
                "roomInstanceId": runtime["roomInstanceId"],
                "secureStorageService": TARGET_SECURE_STORAGE_SERVICE,
                "senderAgentInstanceId": runtime["senderAgentInstanceId"],
            },
            ensure_ascii=False,
            indent=2,
        )
    )


def security() -> None:
    environment = prepare_environment()
    build_runtime_binaries(("agent-room-control-plane",))
    with IsolatedInfrastructure():
        initialize_isolated_dependencies()
        redactor = LogRedactor(environment)
        with ProcessStack() as processes:
            start_control_plane(
                processes, environment, redactor, SECURITY_LOG_ROOT
            )
            start_web(
                processes,
                redactor,
                SECURITY_LOG_ROOT,
                {"VITE_AGENT_ROOM_VERTICAL_SECURITY_DRIVER": "enabled"},
            )
            verify_browser_security(environment)
        scanned_logs = verify_sanitized_logs(
            (
                SECURITY_LOG_ROOT / "control-plane.log",
                SECURITY_LOG_ROOT / "web.log",
            ),
            redactor,
        )
    print(
        json.dumps(
            {
                "devices": "3",
                "logFilesScanned": str(len(scanned_logs)),
                "scenario": "cross-signing+sas+recovery",
            },
            ensure_ascii=False,
            indent=2,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action", choices=("bootstrap", "security"), nargs="?", default="bootstrap"
    )
    arguments = parser.parse_args()
    try:
        if arguments.action == "bootstrap":
            bootstrap()
        elif arguments.action == "security":
            security()
    except (
        LocalRuntimeError,
        McpClientFailure,
        OSError,
        VerticalFailure,
        subprocess.SubprocessError,
    ) as error:
        print(f"纵向验收失败：{error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    configure_console_encoding()
    raise SystemExit(main())
