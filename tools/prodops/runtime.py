"""生产部署编排、主机预检、健康检查与联邦诊断。"""

from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time
from typing import Final
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from .config import DeploymentConfig
from .render import DeploymentPaths, render_deployment
from .secrets import SecretStore


ROOT: Final = Path(__file__).resolve().parents[2]
COMPOSE_FILE: Final = ROOT / "infra" / "production" / "compose.yaml"
SYNAPSE_IMAGE: Final = "matrixdotorg/synapse:v1.159.0"
MINIMUM_MEMORY_BYTES: Final = 4 * 1024**3
RECOMMENDED_MEMORY_BYTES: Final = 8 * 1024**3
MINIMUM_DISK_BYTES: Final = 20 * 1024**3
RECOMMENDED_DISK_BYTES: Final = 100 * 1024**3


class ProductionRuntimeError(RuntimeError):
    """表示主机、Compose 或公开入口未达到生产门禁。"""


@dataclass(frozen=True, slots=True)
class PreflightReport:
    memory_bytes: int | None
    free_disk_bytes: int
    resolved_domains: tuple[str, ...]
    warnings: tuple[str, ...]


@dataclass(slots=True)
class ProductionRuntime:
    config: DeploymentConfig
    paths: DeploymentPaths

    @property
    def secrets(self) -> SecretStore:
        return SecretStore(self.paths.secrets)

    def prepare(self, *, generate_signing_key: bool) -> None:
        self.paths.prepare()
        self.secrets.initialize()
        if generate_signing_key:
            self.ensure_synapse_signing_key()
        render_deployment(self.config, self.paths, self.secrets)

    def ensure_synapse_signing_key(self) -> Path:
        signing_key = self.paths.data / "synapse" / f"{self.config.public.server_name}.signing.key"
        if signing_key.is_file() and signing_key.stat().st_size > 0:
            signing_key.chmod(0o600)
            return signing_key
        self._require_command("docker")
        self.paths.data.joinpath("synapse").mkdir(mode=0o700, parents=True, exist_ok=True)
        self._run(
            [
                "docker",
                "run",
                "--rm",
                "--env",
                f"SYNAPSE_SERVER_NAME={self.config.public.server_name}",
                "--env",
                "SYNAPSE_REPORT_STATS=no",
                "--volume",
                f"{self.paths.data.joinpath('synapse').as_posix()}:/data",
                SYNAPSE_IMAGE,
                "generate",
            ]
        )
        if not signing_key.is_file() or signing_key.stat().st_size == 0:
            raise ProductionRuntimeError("Synapse 容器没有生成 signing key。")
        signing_key.chmod(0o600)
        for generated_name in ("homeserver.yaml", f"{self.config.public.server_name}.log.config"):
            self.paths.data.joinpath("synapse", generated_name).unlink(missing_ok=True)
        return signing_key

    def preflight(self, *, require_linux: bool, require_dns: bool, require_ports: bool) -> PreflightReport:
        if require_linux and not sys.platform.startswith("linux"):
            raise ProductionRuntimeError("生产安装只允许在 Linux 主机执行；Windows/macOS 仅支持渲染和校验。")
        self._require_command("docker")
        self._run(["docker", "compose", "version"], capture=True)

        free_disk = shutil.disk_usage(self.paths.state.parent).free
        if free_disk < MINIMUM_DISK_BYTES:
            raise ProductionRuntimeError("可用磁盘不足 20 GiB，拒绝生产安装。")
        warnings: list[str] = []
        if free_disk < RECOMMENDED_DISK_BYTES:
            warnings.append("可用磁盘低于建议的 100 GiB。")

        memory = _linux_memory_bytes()
        if memory is not None and memory < MINIMUM_MEMORY_BYTES:
            raise ProductionRuntimeError("物理内存不足 4 GiB，拒绝生产安装。")
        if memory is not None and memory < RECOMMENDED_MEMORY_BYTES:
            warnings.append("物理内存低于建议的 8 GiB。")

        domains: list[str] = []
        if require_dns:
            for domain in (
                self.config.public.server_name,
                self.config.public.app_domain,
                self.config.public.api_domain,
                self.config.public.matrix_domain,
                self.config.public.identity_domain,
            ):
                try:
                    socket.getaddrinfo(domain, 443, type=socket.SOCK_STREAM)
                except socket.gaierror as error:
                    raise ProductionRuntimeError(f"DNS 尚未解析：{domain}。") from error
                domains.append(domain)

        if require_ports:
            _assert_port_available(80)
            _assert_port_available(443)
        return PreflightReport(memory, free_disk, tuple(domains), tuple(warnings))

    def validate_compose(self) -> None:
        self._run([*self.compose_command(), "config", "--quiet"])

    def install(self) -> None:
        report = self.preflight(require_linux=True, require_dns=True, require_ports=True)
        for warning in report.warnings:
            print(f"警告：{warning}")
        self.prepare(generate_signing_key=True)
        self.validate_compose()
        self._run([*self.compose_command(), "build", "identity", "control-plane", "gateway"])
        self._run([*self.compose_command(), "pull", "--ignore-buildable"])
        if self.config.database.mode == "embedded":
            self._run([*self.compose_command(), "up", "--detach", "--wait", "postgres"])
        else:
            _wait_tcp(self.config.database.host, self.config.database.port, timeout_seconds=60)
        self.initialize_object_store()
        self._run([*self.compose_command(), "run", "--rm", "migrate"])
        self._run([*self.compose_command(), "up", "--detach", "--wait"])
        self.health(timeout_seconds=180)
        self.federation(timeout_seconds=60)

    def upgrade(self) -> None:
        self.prepare(generate_signing_key=True)
        self.validate_compose()
        self._run([*self.compose_command(), "build", "identity", "control-plane", "gateway"])
        self._run([*self.compose_command(), "pull", "--ignore-buildable"])
        self.initialize_object_store()
        self._run([*self.compose_command(), "run", "--rm", "migrate"])
        self._run([*self.compose_command(), "up", "--detach", "--wait", "--remove-orphans"])
        self.health(timeout_seconds=180)

    def down(self) -> None:
        self._run([*self.compose_command(), "down", "--remove-orphans"])

    def initialize_object_store(self) -> None:
        if self.config.object_store.mode == "embedded":
            self._run([*self.compose_command(), "up", "--detach", "object-store"])
        self._run([*self.compose_command(), "run", "--rm", "object-store-init"])

    def health(self, *, timeout_seconds: int) -> None:
        endpoints = {
            "Web": f"{self.config.public.app_origin}/_agent-room/healthz",
            "Control Plane": f"{self.config.public.api_origin}/health/ready",
            "Matrix": f"{self.config.public.matrix_origin}/_matrix/client/versions",
            "OIDC": f"{self.config.public.identity_origin}/realms/agent-room/.well-known/openid-configuration",
        }
        failures = _wait_http_endpoints(endpoints, timeout_seconds)
        if failures:
            detail = "; ".join(f"{name}: {reason}" for name, reason in failures.items())
            raise ProductionRuntimeError(f"公开健康检查失败：{detail}")

    def federation(self, *, timeout_seconds: int) -> None:
        server_document = _read_json(
            f"https://{self.config.public.server_name}/.well-known/matrix/server",
            timeout_seconds,
        )
        expected_server = f"{self.config.public.matrix_domain}:443"
        if server_document.get("m.server") != expected_server:
            raise ProductionRuntimeError("Matrix server 委派与部署配置不一致。")
        client_document = _read_json(
            f"https://{self.config.public.server_name}/.well-known/matrix/client",
            timeout_seconds,
        )
        homeserver = client_document.get("m.homeserver")
        if not isinstance(homeserver, dict) or homeserver.get("base_url") != self.config.public.matrix_origin:
            raise ProductionRuntimeError("Matrix client 委派与部署配置不一致。")
        version = _read_json(
            f"{self.config.public.matrix_origin}/_matrix/federation/v1/version",
            timeout_seconds,
        )
        if not isinstance(version.get("server"), dict):
            raise ProductionRuntimeError("Matrix 联邦版本入口没有返回 server 对象。")

    def compose_command(self) -> list[str]:
        return [
            "docker",
            "compose",
            "--project-name",
            self.config.project_name,
            "--env-file",
            str(self.paths.compose_environment),
            "--file",
            str(COMPOSE_FILE),
            "--file",
            str(self.paths.worker_override),
        ]

    @staticmethod
    def _require_command(name: str) -> None:
        if shutil.which(name) is None:
            raise ProductionRuntimeError(f"缺少必需命令：{name}。")

    @staticmethod
    def _run(command: list[str], *, capture: bool = False) -> str:
        result = subprocess.run(
            command,
            cwd=ROOT,
            check=False,
            text=True,
            capture_output=capture,
        )
        if result.returncode != 0:
            detail = result.stderr.strip() if capture else f"退出码 {result.returncode}"
            raise ProductionRuntimeError(f"命令执行失败（{command[0]}）：{detail}")
        return result.stdout if capture else ""


def _linux_memory_bytes() -> int | None:
    path = Path("/proc/meminfo")
    if not path.is_file():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("MemTotal:"):
            fields = line.split()
            if len(fields) >= 2 and fields[1].isdigit():
                return int(fields[1]) * 1024
    return None


def _assert_port_available(port: int) -> None:
    for family, address in (
        (socket.AF_INET, ("0.0.0.0", port)),
        (socket.AF_INET6, ("::", port)),
    ):
        listener = socket.socket(family, socket.SOCK_STREAM)
        try:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(address)
        except OSError as error:
            raise ProductionRuntimeError(f"公网端口 {port} 已被占用。") from error
        finally:
            listener.close()


def _wait_tcp(host: str, port: int, *, timeout_seconds: int) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error = "未尝试"
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=3):
                return
        except OSError as error:
            last_error = str(error)
            time.sleep(2)
    raise ProductionRuntimeError(f"PostgreSQL 不可达：{last_error}")


def _wait_http_endpoints(endpoints: dict[str, str], timeout_seconds: int) -> dict[str, str]:
    deadline = time.monotonic() + timeout_seconds
    pending = dict(endpoints)
    failures: dict[str, str] = {}
    while pending and time.monotonic() < deadline:
        for name, url in tuple(pending.items()):
            try:
                request = Request(url, headers={"User-Agent": "agent-room-health/1"})
                with urlopen(request, timeout=5) as response:
                    if 200 <= response.status < 300:
                        pending.pop(name)
                        failures.pop(name, None)
                    else:
                        failures[name] = f"HTTP {response.status}"
            except (HTTPError, URLError, TimeoutError, OSError) as error:
                failures[name] = str(error)
        if pending:
            time.sleep(3)
    return {name: failures.get(name, "超时") for name in pending}


def _read_json(url: str, timeout_seconds: int) -> dict[str, object]:
    request = Request(url, headers={"User-Agent": "agent-room-federation-check/1"})
    try:
        with urlopen(request, timeout=timeout_seconds) as response:
            if response.status != 200:
                raise ProductionRuntimeError(f"联邦入口返回 HTTP {response.status}：{url}")
            value = json.loads(response.read().decode("utf-8"))
    except (HTTPError, URLError, TimeoutError, OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ProductionRuntimeError(f"无法读取联邦入口：{url}") from error
    if not isinstance(value, dict):
        raise ProductionRuntimeError(f"联邦入口不是 JSON 对象：{url}")
    return value
