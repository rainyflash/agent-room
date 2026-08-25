#!/usr/bin/env python3
"""运行任务 34 的可重复故障恢复门禁并生成机器可读报告。"""

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


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT: Final = ROOT / "artifacts" / "reliability" / "task-34-report.json"


@dataclass(frozen=True)
class Scenario:
    name: str
    fault: str
    command: tuple[str, ...]


@dataclass(frozen=True)
class ScenarioResult:
    name: str
    fault: str
    passed: bool
    duration_seconds: float
    log: str


SCENARIOS: Final = (
    Scenario(
        name="浏览器断网与旧协议写入门",
        fault="network_and_stale_service_worker",
        command=(
            "corepack",
            "pnpm@10.28.0",
            "exec",
            "vitest",
            "run",
            "apps/web/src/features/updates",
            "apps/web/src/features/health/domain/degradation-policy.spec.ts",
            "apps/web/src/features/session/domain/session-machine.spec.ts",
            "apps/web/src/features/messages/ui/message-composer.spec.tsx",
        ),
    ),
    Scenario(
        name="Bridge 断网、未知提交与同步缺口",
        fault="network_restart_and_unknown_commit",
        command=(
            "cargo",
            "test",
            "-p",
            "agent-room-bridge-core",
            "--test",
            "message_publication_flow",
            "--test",
            "message_sync_flow",
            "--test",
            "session_flow",
        ),
    ),
    Scenario(
        name="本地状态跨进程恢复",
        fault="process_restart",
        command=(
            "cargo",
            "test",
            "-p",
            "agent-room-bridge-storage-adapter",
            "--test",
            "message_submissions",
            "--test",
            "message_projection",
        ),
    ),
    Scenario(
        name="休眠唤醒与崩溃预算",
        fault="sleep_resume_and_process_crash",
        command=(
            "cargo",
            "test",
            "-p",
            "agent-room-desktop",
        ),
    ),
    Scenario(
        name="孤儿内容失败隔离与重试",
        fault="object_store_temporarily_unavailable",
        command=(
            "cargo",
            "test",
            "-p",
            "agent-room-application",
            "--test",
            "content_lifecycle_flow",
        ),
    ),
)


def run_scenario(scenario: Scenario, log_directory: Path) -> ScenarioResult:
    started = time.monotonic()
    executable = resolve_executable(scenario.command[0])
    result = subprocess.run(
        (executable, *scenario.command[1:]),
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    duration = round(time.monotonic() - started, 3)
    log_path = log_directory / f"{scenario.fault}.log"
    log_path.write_text(result.stdout + result.stderr, encoding="utf-8")
    return ScenarioResult(
        name=scenario.name,
        fault=scenario.fault,
        passed=result.returncode == 0,
        duration_seconds=duration,
        log=str(log_path.relative_to(ROOT)).replace("\\", "/"),
    )


def resolve_executable(name: str) -> str:
    candidates = (f"{name}.cmd", f"{name}.exe", name) if os.name == "nt" else (name,)
    for candidate in candidates:
        resolved = shutil.which(candidate)
        if resolved is not None:
            return resolved
    raise RuntimeError(f"缺少可靠性门禁依赖：{name}")


def main() -> int:
    report_path = REPORT
    report_path.parent.mkdir(parents=True, exist_ok=True)
    log_directory = report_path.parent / "logs"
    log_directory.mkdir(parents=True, exist_ok=True)

    results: list[ScenarioResult] = []
    for scenario in SCENARIOS:
        print(f"[可靠性] {scenario.name} ...", flush=True)
        result = run_scenario(scenario, log_directory)
        results.append(result)
        print("  通过" if result.passed else f"  失败：{result.log}", flush=True)

    passed = all(result.passed for result in results)
    report = {
        "generatedAt": datetime.now(UTC).isoformat(),
        "passed": passed,
        "scenarios": [asdict(result) for result in results],
    }
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"报告：{report_path}")
    return 0 if passed else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("可靠性验证已中断。", file=sys.stderr)
        raise SystemExit(130) from None
