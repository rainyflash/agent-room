#!/usr/bin/env python3
"""编排 Agent Room 内部纵向切片的真实服务、浏览器与本地进程。"""

from __future__ import annotations

import argparse
from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
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
from typing import Final, TextIO
from urllib.error import URLError
from urllib.request import Request, urlopen
import uuid

if __package__:
    from .local_runtime import (
        LocalRuntimeError,
        control_plane_runtime_environment,
        read_environment,
        required_value,
    )
else:
    from local_runtime import (
        LocalRuntimeError,
        control_plane_runtime_environment,
        read_environment,
        required_value,
    )


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


def bootstrap_agent(environment: Mapping[str, str]) -> dict[str, str]:
    VERTICAL_ROOT.mkdir(parents=True, exist_ok=True)
    BOOTSTRAP_RESULT.unlink(missing_ok=True)
    process_environment = os.environ.copy()
    control_plane_environment = control_plane_runtime_environment(
        environment, enable_telemetry=True
    )
    redactor = LogRedactor(environment)
    with ProcessStack() as processes:
        control_plane = processes.start(
            ManagedProcess(
                name="control-plane",
                command=[str(runtime_binary("agent-room-control-plane"))],
                environment=control_plane_environment,
                log_path=LOG_ROOT / "control-plane.log",
                redactor=redactor,
            )
        )
        wait_for_http(
            "http://127.0.0.1:8090/health/live",
            control_plane,
            timeout_seconds=120,
        )

        node = executable("node")
        web = processes.start(
            ManagedProcess(
                name="web",
                command=[
                    node,
                    "apps/web/node_modules/vite/bin/vite.js",
                    "apps/web",
                    "--host",
                    "0.0.0.0",
                    "--port",
                    "5173",
                    "--strictPort",
                ],
                environment=process_environment,
                log_path=LOG_ROOT / "web.log",
                redactor=redactor,
            )
        )
        wait_for_http("http://127.0.0.1:5173/connect", web, timeout_seconds=60)

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
                node,
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
        agent = bootstrap_agent(environment)
    print(
        json.dumps(
            {
                "agentId": agent["agentId"],
                "catalogId": catalog_id,
                "principalId": agent["principalId"],
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
