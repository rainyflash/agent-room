#!/usr/bin/env python3
"""编排 Agent Room 内部纵向切片的真实服务、浏览器与本地进程。"""

from __future__ import annotations

import argparse
from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
import ctypes
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from typing import Final, Protocol, TextIO
from urllib.error import URLError
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
CATALOG_SEED_ID: Final = "019d2c44-1dc5-7a5b-9e32-2f3c1d4b5a61"
CATALOG_SLUG: Final = "vertical-codex-lobby"
SECURE_STORAGE_SERVICE: Final = "dev.agent-room.bridge.vertical-24"
BRIDGE_DATA_ROOT: Final = VERTICAL_ROOT / "bridge"
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


LineObserver = Callable[[str], None]


class ProcessHealth(Protocol):
    def ensure_running(self) -> None: ...


class BridgeRuntimeObservation:
    """只在内存中捕获一次性设备码，并暴露可等待的运行时里程碑。"""

    def __init__(self) -> None:
        self._device_code: str | None = None
        self._device_code_ready = threading.Event()
        self._agent_online = threading.Event()
        self._lock = threading.Lock()

    def observe(self, line: str) -> None:
        match = DEVICE_CODE_LINE.fullmatch(line.strip())
        if match is not None:
            with self._lock:
                self._device_code = match.group(2)
            self._device_code_ready.set()
        if "Agent 已进入公共大厅并开始同步。" in line:
            self._agent_online.set()

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
        self, process: ProcessHealth, *, timeout_seconds: float
    ) -> None:
        self._wait(
            self._agent_online,
            process,
            timeout_seconds=timeout_seconds,
            label="Agent 自动入厅",
        )

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


class IsolatedBridgeState(AbstractContextManager["IsolatedBridgeState"]):
    """只清理纵向验收专属目录和凭据命名空间。"""

    def __enter__(self) -> "IsolatedBridgeState":
        reset_bridge_data()
        clear_vertical_secure_storage()
        BRIDGE_DATA_ROOT.mkdir(parents=True, exist_ok=True)
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        failures: list[str] = []
        try:
            reset_bridge_data()
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


def reset_bridge_data() -> None:
    target = BRIDGE_DATA_ROOT.resolve()
    expected_parent = VERTICAL_ROOT.resolve()
    if target.parent != expected_parent or target.name != "bridge":
        raise VerticalFailure("拒绝清理未经审计的 Bridge 测试目录。")
    if target.exists():
        shutil.rmtree(target)


def clear_vertical_secure_storage() -> None:
    if os.name == "nt":
        for account in SECURE_STORAGE_ACCOUNTS:
            delete_windows_credential(
                windows_credential_target(SECURE_STORAGE_SERVICE, account)
            )
        return
    if sys.platform == "darwin":
        for account in SECURE_STORAGE_ACCOUNTS:
            completed = subprocess.run(
                [
                    executable("security"),
                    "delete-generic-password",
                    "-s",
                    SECURE_STORAGE_SERVICE,
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
    for account in SECURE_STORAGE_ACCOUNTS:
        completed = subprocess.run(
            [
                secret_tool,
                "clear",
                "service",
                SECURE_STORAGE_SERVICE,
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


def build_runtime_binaries() -> None:
    run_checked(
        [
            executable("cargo"),
            "build",
            "--locked",
            "-p",
            "agent-room-control-plane",
            "-p",
            "agent-room-bridge",
            "-p",
            "agent-room-codex-mcp",
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
        except (TimeoutError, URLError):
            pass
        time.sleep(0.4)
    raise VerticalFailure(f"服务 {url} 未在 {timeout_seconds:.0f} 秒内就绪。")


def http_status_is_ready(status: int) -> bool:
    """只把成功响应视为就绪；重定向和客户端错误都必须继续等待。"""
    return 200 <= status < 300


def start_control_plane(
    processes: ProcessStack,
    environment: Mapping[str, str],
    redactor: LogRedactor,
) -> ManagedProcess:
    control_plane = processes.start(
        ManagedProcess(
            name="control-plane",
            command=[str(runtime_binary("agent-room-control-plane"))],
            environment=control_plane_runtime_environment(
                environment, enable_telemetry=True
            ),
            log_path=LOG_ROOT / "control-plane.log",
            redactor=redactor,
        )
    )
    wait_for_http(
        "http://127.0.0.1:8090/health/live",
        control_plane,
        timeout_seconds=120,
    )
    return control_plane


def start_web(processes: ProcessStack, redactor: LogRedactor) -> ManagedProcess:
    web = processes.start(
        ManagedProcess(
            name="web",
            command=[
                executable("node"),
                "apps/web/node_modules/vite/bin/vite.js",
                "apps/web",
                "--host",
                "0.0.0.0",
                "--port",
                "5173",
                "--strictPort",
            ],
            environment=os.environ.copy(),
            log_path=LOG_ROOT / "web.log",
            redactor=redactor,
        )
    )
    wait_for_http("http://127.0.0.1:5173/connect", web, timeout_seconds=60)
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


def start_authorized_bridge(
    *,
    processes: ProcessStack,
    environment: Mapping[str, str],
    catalog_id: str,
    agent_id: str,
    redactor: LogRedactor,
) -> tuple[ManagedProcess, dict[str, str]]:
    bridge_environment = bridge_runtime_environment(
        data_root=BRIDGE_DATA_ROOT.resolve(),
        agent_id=agent_id,
        public_lobby_catalog_id=catalog_id,
        secure_storage_service=SECURE_STORAGE_SERVICE,
    )
    observation = BridgeRuntimeObservation()
    bridge = processes.start(
        ManagedProcess(
            name="bridge",
            command=[str(runtime_binary("agent-room-bridge"))],
            environment=bridge_environment,
            log_path=LOG_ROOT / "bridge.log",
            redactor=redactor,
            on_line=observation.observe,
        )
    )
    device_code = observation.wait_for_device_code(bridge, timeout_seconds=90)
    approve_device_grant(environment, device_code)
    observation.wait_for_agent_online(bridge, timeout_seconds=180)
    return bridge, bridge_environment


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


def verify_mcp_identity(
    *,
    bridge_environment: Mapping[str, str],
    expected_agent_id: str,
    redactor: LogRedactor,
) -> dict[str, str]:
    with McpStdioClient(
        command=[str(runtime_binary("agent-room-codex-mcp"))],
        working_directory=ROOT,
        environment=bridge_environment,
        stderr_path=LOG_ROOT / "codex-mcp.log",
        sanitize_line=redactor.redact,
    ) as client:
        tools = frozenset(client.list_tool_names())
        if tools != EXPECTED_MCP_TOOLS:
            missing = sorted(EXPECTED_MCP_TOOLS - tools)
            unexpected = sorted(tools - EXPECTED_MCP_TOOLS)
            raise VerticalFailure(
                f"MCP 工具面不匹配；缺少 {missing}，多出 {unexpected}。"
            )
        response = client.call_tool("agent_room_get_self", {})

    if response.get("type") != "self_summary":
        raise VerticalFailure("MCP 当前身份返回了错误响应类型。")
    summary = require_object(response.get("summary"), "MCP 当前身份摘要")
    agent = require_object(summary.get("agent"), "MCP Agent 身份")
    if agent.get("agentId") != expected_agent_id:
        raise VerticalFailure("MCP Agent 身份与浏览器创建结果不一致。")
    instance_id = require_text(summary.get("instanceId"), "MCP Agent 实例标识")
    require_uuid_v7(instance_id, "MCP Agent 实例标识")
    room = active_room_for_agent(expected_agent_id)
    return {
        "agentInstanceId": instance_id,
        "matrixDeviceId": require_text(
            summary.get("matrixDeviceId"), "MCP Matrix 设备标识"
        ),
        "roomInstanceId": room["roomInstanceId"],
        "matrixRoomId": room["matrixRoomId"],
    }


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


def require_uuid_v7(value: str, label: str) -> None:
    if not UUID_V7_TEXT.fullmatch(value):
        raise VerticalFailure(f"{label} 不是 UUIDv7。")
    try:
        parsed = uuid.UUID(value)
    except ValueError as error:
        raise VerticalFailure(f"{label} 不是有效 UUID。") from error
    if parsed.version != 7:
        raise VerticalFailure(f"{label} 不是 UUIDv7。")


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
            agent = bootstrap_agent(environment)
            _, bridge_environment = start_authorized_bridge(
                processes=processes,
                environment=environment,
                catalog_id=catalog_id,
                agent_id=agent["agentId"],
                redactor=redactor,
            )
            runtime = verify_mcp_identity(
                bridge_environment=bridge_environment,
                expected_agent_id=agent["agentId"],
                redactor=redactor,
            )
    print(
        json.dumps(
            {
                "agentId": agent["agentId"],
                "agentInstanceId": runtime["agentInstanceId"],
                "catalogId": catalog_id,
                "matrixRoomId": runtime["matrixRoomId"],
                "principalId": agent["principalId"],
                "roomInstanceId": runtime["roomInstanceId"],
                "secureStorageService": SECURE_STORAGE_SERVICE,
            },
            ensure_ascii=False,
            indent=2,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("bootstrap",), nargs="?", default="bootstrap")
    arguments = parser.parse_args()
    try:
        if arguments.action == "bootstrap":
            bootstrap()
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
