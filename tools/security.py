#!/usr/bin/env python3
"""运行任务 35 的封闭安全门禁并生成机器可读报告。"""

from __future__ import annotations

from dataclasses import asdict, dataclass
from datetime import UTC, datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from typing import Final

if __package__:
    from .sensitive_output import scan_files
else:
    from sensitive_output import scan_files


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT: Final = ROOT / "artifacts" / "security" / "task-35-report.json"
CANARY_SECRET: Final = "task-35-canary-secret"


@dataclass(frozen=True)
class Scenario:
    name: str
    control: str
    command: tuple[str, ...]


@dataclass(frozen=True)
class ScenarioResult:
    name: str
    control: str
    passed: bool
    duration_seconds: float
    log: str


SCENARIOS: Final = (
    Scenario(
        "IPC 帧与方法边界模糊测试",
        "ipc_fuzz",
        ("cargo", "test", "-p", "agent-room-bridge-ipc"),
    ),
    Scenario(
        "A2A Card、HTTPS 与 SSRF 边界",
        "a2a_fuzz_ssrf",
        ("cargo", "test", "-p", "agent-room-a2a-adapter"),
    ),
    Scenario(
        "Matrix 事件解析模糊测试",
        "matrix_event_fuzz",
        ("cargo", "test", "-p", "agent-room-bridge-core", "--lib"),
    ),
    Scenario(
        "MCP Schema 与 IPC 二次校验",
        "mcp_schema_fuzz",
        ("cargo", "test", "-p", "agent-room-codex-mcp"),
    ),
    Scenario(
        "CSRF、同源与请求体限制",
        "control_plane_web_security",
        ("cargo", "test", "-p", "agent-room-control-plane"),
    ),
    Scenario(
        "附件扫描与内容生命周期策略",
        "attachment_policy",
        ("cargo", "test", "-p", "agent-room-content-adapter", "--lib"),
    ),
    Scenario(
        "内容授权、扫描失败与分层限流",
        "content_authorization_rate_limits",
        (
            "cargo",
            "test",
            "-p",
            "agent-room-application",
            "--test",
            "content_upload_flow",
            "--test",
            "content_read_flow",
            "--test",
            "moderation_flow",
        ),
    ),
    Scenario(
        "受限富文本与远端提示注入隔离",
        "prompt_injection_and_content_parser",
        (
            "corepack",
            "pnpm@10.28.0",
            "exec",
            "vitest",
            "run",
            "apps/web/src/features/messages/ui/restricted-markdown.spec.tsx",
            "apps/web/src/features/messages/ui/content-inspector.spec.tsx",
        ),
    ),
    Scenario(
        "Tauri 能力与桌面 CSP",
        "tauri_capabilities_csp",
        ("cargo", "test", "-p", "agent-room-desktop", "capability_tests"),
    ),
    Scenario(
        "五类敏感输出与真实日志脱敏",
        "sensitive_output_scan",
        (
            "python",
            "-m",
            "unittest",
            "tools.tests.test_sensitive_output",
            "tools.tests.test_vertical",
        ),
    ),
    Scenario(
        "仓库秘密扫描",
        "repository_secret_scan",
        ("corepack", "pnpm@10.28.0", "secrets:check"),
    ),
)


def run_scenario(scenario: Scenario, log_directory: Path) -> ScenarioResult:
    started = time.monotonic()
    executable = resolve_executable(scenario.command[0])
    environment = os.environ.copy()
    environment["AGENT_ROOM_SECURITY_CANARY"] = CANARY_SECRET
    result = subprocess.run(
        (executable, *scenario.command[1:]),
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    duration = round(time.monotonic() - started, 3)
    log_path = log_directory / f"{scenario.control}.log"
    log_path.write_text(result.stdout + result.stderr, encoding="utf-8")
    return ScenarioResult(
        name=scenario.name,
        control=scenario.control,
        passed=result.returncode == 0,
        duration_seconds=duration,
        log=relative(log_path),
    )


def verify_static_security_policy(log_directory: Path) -> ScenarioResult:
    started = time.monotonic()
    failures: list[str] = []
    tauri = json.loads(
        (ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json").read_text(
            encoding="utf-8"
        )
    )
    security = tauri["app"]["security"]
    csp = security["csp"]
    required_csp = (
        "script-src 'self' 'wasm-unsafe-eval'",
        "object-src 'none'",
        "frame-src 'none'",
        "frame-ancestors 'none'",
        "base-uri 'none'",
        "upgrade-insecure-requests",
    )
    failures.extend(f"tauri-csp:{value}" for value in required_csp if value not in csp)
    if "'unsafe-eval'" in csp.split("script-src", 1)[1].split(";", 1)[0].split():
        failures.append("tauri-csp:unsafe-eval")

    capability = json.loads(
        (ROOT / "apps" / "desktop" / "src-tauri" / "capabilities" / "main.json").read_text(
            encoding="utf-8"
        )
    )
    permissions = capability.get("permissions", [])
    if any(
        not isinstance(permission, str)
        or "*" in permission
        or permission.startswith(("fs:", "shell:", "process:"))
        for permission in permissions
    ):
        failures.append("tauri-capability:wildcard-or-ambient")

    caddy = (ROOT / "infra" / "caddy" / "Caddyfile").read_text(encoding="utf-8")
    failures.extend(f"web-csp:{value}" for value in required_csp if value not in caddy)
    for header in (
        "Permissions-Policy",
        "Referrer-Policy \"no-referrer\"",
        "X-Content-Type-Options \"nosniff\"",
    ):
        if header not in caddy:
            failures.append(f"web-header:{header}")

    renderer = (
        ROOT / "apps" / "web" / "src" / "features" / "messages" / "ui" / "restricted-markdown.tsx"
    ).read_text(encoding="utf-8")
    for forbidden in (
        "dangerouslySetInnerHTML",
        "HandoffGateway",
        "MessagePublisher",
        "window.",
        "document.",
    ):
        if forbidden in renderer:
            failures.append(f"rich-text-renderer:{forbidden}")

    ssrf_client = (ROOT / "crates" / "a2a-adapter" / "src" / "http.rs").read_text(
        encoding="utf-8"
    )
    for required in ("https_only(true)", "redirect(Policy::none())", "resolve_to_addrs"):
        if required not in ssrf_client:
            failures.append(f"ssrf:{required}")

    log_path = log_directory / "static_security_policy.log"
    log_path.write_text(
        "安全配置通过。\n" if not failures else "\n".join(sorted(failures)) + "\n",
        encoding="utf-8",
    )
    return ScenarioResult(
        name="CSP、能力、富文本与 SSRF 静态策略",
        control="static_security_policy",
        passed=not failures,
        duration_seconds=round(time.monotonic() - started, 3),
        log=relative(log_path),
    )


def resolve_executable(name: str) -> str:
    candidates = (f"{name}.cmd", f"{name}.exe", name) if os.name == "nt" else (name,)
    for candidate in candidates:
        resolved = shutil.which(candidate)
        if resolved is not None:
            return resolved
    raise RuntimeError(f"缺少安全门禁依赖：{name}")


def relative(path: Path) -> str:
    return str(path.relative_to(ROOT)).replace("\\", "/")


def main() -> int:
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    log_directory = REPORT.parent / "logs"
    log_directory.mkdir(parents=True, exist_ok=True)

    results = [verify_static_security_policy(log_directory)]
    for scenario in SCENARIOS:
        print(f"[安全] {scenario.name} ...", flush=True)
        result = run_scenario(scenario, log_directory)
        results.append(result)
        print("  通过" if result.passed else f"  失败：{result.log}", flush=True)

    log_files = tuple(
        ("log", ROOT / result.log)
        for result in results
        if result.control != "sensitive_output_scan"
    )
    sensitive_violations = scan_files(log_files, known_secrets=(CANARY_SECRET,))
    passed = all(result.passed for result in results) and not sensitive_violations
    report = {
        "generatedAt": datetime.now(UTC).isoformat(),
        "passed": passed,
        "sensitiveOutputViolations": [asdict(item) for item in sensitive_violations],
        "scenarios": [asdict(result) for result in results],
    }
    REPORT.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"报告：{REPORT}")
    return 0 if passed else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("安全验证已中断。", file=sys.stderr)
        raise SystemExit(130) from None
