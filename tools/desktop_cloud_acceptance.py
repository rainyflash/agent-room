#!/usr/bin/env python3
"""在真实 Windows Tauri 进程中验收 Bridge 离线时的云端产品闭环。"""

from __future__ import annotations

import argparse
from collections.abc import Sequence
import os
from pathlib import Path
import socket
import sys
import tempfile
from typing import Final
import uuid

if __package__:
    from .local_runtime import required_value
    from .vertical import (
        LogRedactor,
        ManagedProcess,
        ProcessStack,
        ROOT,
        SECURE_STORAGE_ACCOUNTS,
        IsolatedInfrastructure,
        VerticalFailure,
        build_runtime_binaries,
        configure_console_encoding,
        delete_windows_credential,
        executable,
        initialize_isolated_dependencies,
        prepare_environment,
        read_string_object,
        run_checked,
        seed_public_catalog,
        start_control_plane,
        start_web,
        verify_sanitized_logs,
        wait_for_http,
        windows_credential_target,
        write_json,
    )
else:
    from local_runtime import required_value
    from vertical import (
        LogRedactor,
        ManagedProcess,
        ProcessStack,
        ROOT,
        SECURE_STORAGE_ACCOUNTS,
        IsolatedInfrastructure,
        VerticalFailure,
        build_runtime_binaries,
        configure_console_encoding,
        delete_windows_credential,
        executable,
        initialize_isolated_dependencies,
        prepare_environment,
        read_string_object,
        run_checked,
        seed_public_catalog,
        start_control_plane,
        start_web,
        verify_sanitized_logs,
        wait_for_http,
        windows_credential_target,
        write_json,
    )


ARTIFACT_ROOT: Final = ROOT / "artifacts" / "desktop-acceptance"
DEFAULT_REPORT: Final = ARTIFACT_ROOT / "desktop-cloud-closure.json"
DESKTOP_BINARY: Final = ROOT / "target" / "debug" / "agent-room-desktop.exe"
PLAYWRIGHT_TEST: Final = "desktop-cloud-closure.e2e.ts"
DESKTOP_ORIGINS: Final = frozenset({"http://tauri.localhost", "tauri://localhost"})
EXPECTED_EVIDENCE: Final = {
    "bridgePhase": "halted",
    "controlPlaneStatus": "online",
    "lobbyEntered": "true",
    "matrixStatus": "online",
    "processKind": "tauri_webview2",
    "tauriRuntimeDetected": "true",
    "workspaceVisible": "true",
}


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--report",
        type=Path,
        default=DEFAULT_REPORT,
        help="脱敏验收结果的输出路径。",
    )
    parser.add_argument(
        "--cdp-port",
        type=int,
        default=0,
        help="WebView2 调试端口；0 表示自动选择本机空闲端口。",
    )
    return parser.parse_args(argv)


def corepack_pnpm_command(*arguments: str) -> list[str]:
    """固定仓库声明的 pnpm 版本，避免全局工具链漂移。"""
    return [executable("corepack"), "pnpm@10.28.0", *arguments]


def tauri_acceptance_build_command() -> list[str]:
    return corepack_pnpm_command(
        "--filter",
        "@agent-room/desktop",
        "exec",
        "tauri",
        "build",
        "--debug",
        "--no-bundle",
        "--config",
        "src-tauri/tauri.sidecar.conf.json",
        "--config",
        "src-tauri/tauri.acceptance.conf.json",
    )


def prepare_sidecar_command() -> list[str]:
    return corepack_pnpm_command(
        "--filter",
        "@agent-room/desktop",
        "run",
        "prepare:sidecar:debug",
    )


def playwright_acceptance_command() -> list[str]:
    return [
        executable("node"),
        "apps/web/node_modules/@playwright/test/cli.js",
        "test",
        "--config",
        "apps/web/playwright.vertical.config.ts",
        PLAYWRIGHT_TEST,
    ]


def build_acceptance_runtime() -> None:
    """构建与验收同一工作树修订对应的控制面、Sidecar 和桌面壳。"""
    build_runtime_binaries()
    run_checked(prepare_sidecar_command())
    build_environment = os.environ.copy()
    build_environment.update(
        {
            "VITE_AGENT_ROOM_CONTROL_PLANE_URL": (
                "https://api.agent-room.localhost:18443"
            ),
            "VITE_AGENT_ROOM_MATRIX_HOMESERVER_URL": (
                "https://matrix.agent-room.localhost:18443"
            ),
        }
    )
    run_checked(
        tauri_acceptance_build_command(),
        environment=build_environment,
    )
    if not DESKTOP_BINARY.is_file():
        raise VerticalFailure(f"桌面验收构建没有生成 {DESKTOP_BINARY}。")


def choose_cdp_port(requested: int) -> int:
    if requested != 0:
        if requested < 1_024 or requested > 65_535:
            raise VerticalFailure("cdp-port 必须为 0 或 1024 到 65535。")
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
                listener.bind(("127.0.0.1", requested))
        except OSError as error:
            raise VerticalFailure(f"cdp-port {requested} 已被占用。") from error
        return requested
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def desktop_acceptance_environment(
    temporary_root: Path,
    *,
    cdp_port: int,
    secure_storage_service: str,
) -> dict[str, str]:
    """隔离桌面进程状态，并故意让原生 Bridge 后端不可达。"""
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("AGENT_ROOM_")
        and name != "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"
    }
    environment.update(
        {
            "AGENT_ROOM_BRIDGE_DATA_DIR": str(temporary_root / "bridge-data"),
            "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE": secure_storage_service,
            "AGENT_ROOM_CONTROL_PLANE_URL": "http://127.0.0.1:1",
            "AGENT_ROOM_BROWSER_CONTROL_PLANE_URL": "http://127.0.0.1:1/",
            "AGENT_ROOM_MATRIX_BASE_URL": "http://127.0.0.1:1",
            "AGENT_ROOM_OIDC_ISSUER_URL": "http://127.0.0.1:1",
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": (
                f"--remote-debugging-port={cdp_port} --ignore-certificate-errors"
            ),
        }
    )
    return environment


def playwright_acceptance_environment(
    *,
    cdp_port: int,
    password: str,
    result_path: Path,
) -> dict[str, str]:
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("AGENT_ROOM_")
    }
    environment.update(
        {
            "AGENT_ROOM_DESKTOP_ACCEPTANCE_CDP_URL": (
                f"http://127.0.0.1:{cdp_port}"
            ),
            "AGENT_ROOM_DESKTOP_ACCEPTANCE_DISPLAY_NAME": "Local Developer",
            "AGENT_ROOM_DESKTOP_ACCEPTANCE_PASSWORD": password,
            "AGENT_ROOM_DESKTOP_ACCEPTANCE_RESULT": str(result_path),
            "AGENT_ROOM_DESKTOP_ACCEPTANCE_USERNAME": "developer",
            "AGENT_ROOM_VERTICAL_EVIDENCE_TASK": (
                "cloud-first-product-closure-desktop"
            ),
        }
    )
    return environment


def validate_evidence(path: Path) -> dict[str, str]:
    evidence = read_string_object(path)
    expected_names = {*EXPECTED_EVIDENCE, "desktopOrigin", "lobbyName"}
    if set(evidence) != expected_names:
        raise VerticalFailure("桌面闭环验收结果字段与受审计契约不一致。")
    for name, expected in EXPECTED_EVIDENCE.items():
        if evidence.get(name) != expected:
            raise VerticalFailure(f"桌面闭环验收结果 {name} 未通过。")
    if evidence.get("desktopOrigin") not in DESKTOP_ORIGINS:
        raise VerticalFailure("桌面闭环验收没有运行在真实 Tauri 来源中。")
    if not evidence.get("lobbyName", "").strip():
        raise VerticalFailure("桌面闭环验收没有进入具体公共大厅。")
    return evidence


def clear_acceptance_credentials(secure_storage_service: str) -> None:
    """清理本次随机命名空间，禁止验收污染用户日常身份。"""
    for account in SECURE_STORAGE_ACCOUNTS:
        delete_windows_credential(
            windows_credential_target(secure_storage_service, account)
        )


def run_acceptance(report: Path, requested_cdp_port: int) -> None:
    if os.name != "nt":
        raise VerticalFailure("真实桌面闭环验收只能在 Windows 上运行。")
    environment = prepare_environment()
    password = required_value(environment, "SEED_ADMIN_PASSWORD")
    build_acceptance_runtime()
    cdp_port = choose_cdp_port(requested_cdp_port)
    secure_storage_service = (
        f"dev.agent-room.desktop-cloud-acceptance.{uuid.uuid4().hex}"
    )
    log_root = ARTIFACT_ROOT / "services"
    local_root = ROOT / ".local"
    local_root.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(
        prefix="desktop-cloud-acceptance-",
        dir=local_root,
    ) as directory:
        temporary_root = Path(directory)
        raw_result = temporary_root / "desktop-cloud-closure.json"
        desktop_environment = desktop_acceptance_environment(
            temporary_root,
            cdp_port=cdp_port,
            secure_storage_service=secure_storage_service,
        )
        redactor = LogRedactor({**environment, **desktop_environment})
        try:
            with IsolatedInfrastructure():
                initialize_isolated_dependencies()
                seed_public_catalog()
                with ProcessStack() as processes:
                    start_control_plane(processes, environment, redactor, log_root)
                    start_web(processes, redactor, log_root)
                    desktop = processes.start(
                        ManagedProcess(
                            name="desktop",
                            command=[str(DESKTOP_BINARY)],
                            environment=desktop_environment,
                            log_path=log_root / "desktop.log",
                            redactor=redactor,
                        )
                    )
                    wait_for_http(
                        f"http://127.0.0.1:{cdp_port}/json/version",
                        desktop,
                        timeout_seconds=60,
                    )
                    run_checked(
                        playwright_acceptance_command(),
                        environment=playwright_acceptance_environment(
                            cdp_port=cdp_port,
                            password=password,
                            result_path=raw_result,
                        ),
                    )
                    evidence = validate_evidence(raw_result)
                verify_sanitized_logs(
                    (
                        log_root / "control-plane.log",
                        log_root / "web.log",
                        log_root / "desktop.log",
                    ),
                    redactor,
                    additional_secrets=(password,),
                )
                write_json(report.resolve(), evidence)
        finally:
            clear_acceptance_credentials(secure_storage_service)


def main(argv: Sequence[str] | None = None) -> int:
    configure_console_encoding()
    args = parse_args(argv)
    try:
        run_acceptance(args.report, args.cdp_port)
    except (OSError, VerticalFailure) as error:
        print(f"桌面云端闭环验收失败：{error}", file=sys.stderr)
        return 1
    print(f"桌面云端闭环验收通过：{args.report.resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
