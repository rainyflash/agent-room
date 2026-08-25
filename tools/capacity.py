#!/usr/bin/env python3
"""运行容量模型、Bridge 常驻观测，并汇总任务 39 的真实证据。"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
import hashlib
import json
import math
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time
from typing import Final, Iterable, Mapping, Sequence


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT_DIRECTORY: Final = ROOT / "artifacts" / "capacity"
MODEL_REPORT: Final = REPORT_DIRECTORY / "model-report.json"
SOAK_REPORT: Final = REPORT_DIRECTORY / "bridge-soak-report.json"
GATE_REPORT: Final = REPORT_DIRECTORY / "task-39-report.json"
SOAK_SECONDS: Final = 72 * 60 * 60
MIB: Final = 1_024 * 1_024


@dataclass(frozen=True)
class CapacityTarget:
    identifier: str
    value: int
    unit: str


TARGETS: Final = (
    CapacityTarget("registered_agents", 10_000, "agents"),
    CapacityTarget("concurrent_instances", 1_000, "instances"),
    CapacityTarget("lobby_members", 250, "members"),
    CapacityTarget("sustained_messages_per_second", 10, "messages_per_second"),
    CapacityTarget("burst_messages_per_second", 50, "messages_per_second"),
    CapacityTarget("attachment_bytes", 25 * MIB, "bytes"),
    CapacityTarget("web_scene_nodes", 200, "nodes"),
    CapacityTarget("bridge_soak_seconds", SOAK_SECONDS, "seconds"),
)

REQUIRED_REAL_SCENARIOS: Final = (
    "database_directory_and_allocation",
    "matrix_lobby_and_messages",
    "content_25_mib_concurrency",
    "federation_outage_backfill",
    "web_200_node_budget",
    "bridge_72_hour_soak",
)


class CapacityFailure(RuntimeError):
    """表示容量证据缺失、损坏或没有达到硬门槛。"""


def percentile(values: Sequence[float], ratio: float) -> float:
    if not values:
        raise CapacityFailure("百分位样本不能为空。")
    if not 0 <= ratio <= 1:
        raise CapacityFailure("百分位比例必须处于 0 到 1。")
    ordered = sorted(values)
    index = min(len(ordered) - 1, math.ceil(len(ordered) * ratio) - 1)
    return ordered[max(0, index)]


def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode != 0:
        raise CapacityFailure("无法读取当前 Git 修订。")
    return result.stdout.strip()


def write_json(path: Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    temporary.replace(path)


def target_map() -> dict[str, int]:
    return {target.identifier: target.value for target in TARGETS}


def run_model(seed: int) -> dict[str, object]:
    """快速暴露算法级退化；该结果永远不能满足真实容量门。"""

    generator = random.Random(seed)
    started = time.perf_counter()
    agents = tuple(
        (f"agent-{index:05d}", f"Build Agent {index:05d}") for index in range(10_000)
    )
    construction_ms = (time.perf_counter() - started) * 1_000

    search_samples: list[float] = []
    search_hits = 0
    for _ in range(120):
        needle = f"{generator.randrange(0, 10_000):04d}"
        search_started = time.perf_counter()
        matches = [agent for agent in agents if needle in agent[0] or needle in agent[1]]
        search_samples.append((time.perf_counter() - search_started) * 1_000)
        search_hits += len(matches)

    allocation_started = time.perf_counter()
    room_loads: list[int] = []
    for _ in range(1_000):
        room_index = next(
            (index for index, load in enumerate(room_loads) if load < 250), None
        )
        if room_index is None:
            room_loads.append(1)
        else:
            room_loads[room_index] += 1
    allocation_ms = (time.perf_counter() - allocation_started) * 1_000

    lease_started = time.perf_counter()
    leases = {f"instance-{index:04d}": 30_000 for index in range(1_000)}
    for tick in range(20):
        leases = {identifier: expiry + tick + 1 for identifier, expiry in leases.items()}
    lease_ms = (time.perf_counter() - lease_started) * 1_000

    payload = bytes((index % 251 for index in range(25 * MIB)))
    digest_started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=4) as executor:
        digests = tuple(executor.map(lambda _: hashlib.sha256(payload).hexdigest(), range(4)))
    digest_ms = (time.perf_counter() - digest_started) * 1_000

    metrics: dict[str, object] = {
        "agentConstructionMilliseconds": round(construction_ms, 3),
        "searchP95Milliseconds": round(percentile(search_samples, 0.95), 3),
        "searchSampleCount": len(search_samples),
        "searchHitCount": search_hits,
        "allocatedInstances": sum(room_loads),
        "allocatedRooms": len(room_loads),
        "maximumRoomLoad": max(room_loads),
        "allocationMilliseconds": round(allocation_ms, 3),
        "leaseRenewals": len(leases) * 20,
        "leaseRenewalMilliseconds": round(lease_ms, 3),
        "concurrentDigestBytes": len(payload) * len(digests),
        "digestMilliseconds": round(digest_ms, 3),
        "digestAgreement": len(set(digests)) == 1,
    }
    passed = (
        len(agents) == 10_000
        and sum(room_loads) == 1_000
        and max(room_loads) == 250
        and len(room_loads) == 4
        and metrics["digestAgreement"] is True
    )
    report: dict[str, object] = {
        "schemaVersion": 1,
        "scenario": "capacity_algorithm_model",
        "evidenceLevel": "model_only",
        "generatedAt": datetime.now(UTC).isoformat(),
        "revision": git_revision(),
        "seed": seed,
        "passed": passed,
        "targets": [asdict(target) for target in TARGETS],
        "metrics": metrics,
        "releaseGateEligible": False,
        "warning": "算法模型不包含真实 PostgreSQL、Matrix、对象存储、浏览器或 Bridge，不能用于发布放行。",
    }
    write_json(MODEL_REPORT, report)
    return report


def process_rss_bytes(process_id: int) -> int:
    if os.name == "nt":
        return windows_process_rss_bytes(process_id)
    status_path = Path("/proc") / str(process_id) / "status"
    if not status_path.is_file():
        raise CapacityFailure("Bridge 进程状态不可读。")
    for line in status_path.read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            fields = line.split()
            return int(fields[1]) * 1_024
    raise CapacityFailure("Bridge 进程没有 VmRSS 指标。")


def windows_process_rss_bytes(process_id: int) -> int:
    import ctypes
    from ctypes import wintypes

    process_query_information = 0x0400
    process_vm_read = 0x0010

    class ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    handle = kernel32.OpenProcess(
        process_query_information | process_vm_read, False, process_id
    )
    if not handle:
        raise CapacityFailure("无法打开 Bridge 进程读取内存。")
    try:
        counters = ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        succeeded = psapi.GetProcessMemoryInfo(
            handle, ctypes.byref(counters), counters.cb
        )
        if not succeeded:
            raise CapacityFailure("无法读取 Bridge 工作集。")
        return int(counters.WorkingSetSize)
    finally:
        kernel32.CloseHandle(handle)


def resolve_executable(name: str) -> str:
    resolved = shutil.which(name)
    if resolved is None and os.name == "nt":
        resolved = shutil.which(f"{name}.exe") or shutil.which(f"{name}.cmd")
    if resolved is None:
        raise CapacityFailure(f"找不到常驻命令：{name}")
    return resolved


def run_bridge_soak(
    command: Sequence[str], duration_seconds: int, sample_seconds: int
) -> dict[str, object]:
    if not command:
        raise CapacityFailure("Bridge 常驻测试必须提供真实启动命令。")
    if duration_seconds <= 0 or sample_seconds <= 0 or sample_seconds > 60:
        raise CapacityFailure("常驻时长必须为正数，采样间隔必须处于 1 到 60 秒。")

    executable = resolve_executable(command[0])
    started_at = datetime.now(UTC)
    started = time.monotonic()
    process = subprocess.Popen(
        [executable, *command[1:]],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    samples: list[dict[str, object]] = []
    try:
        while True:
            elapsed = time.monotonic() - started
            return_code = process.poll()
            if return_code is not None:
                raise CapacityFailure(
                    f"Bridge 在 {elapsed:.1f} 秒后退出，退出码为 {return_code}。"
                )
            rss = process_rss_bytes(process.pid)
            samples.append(
                {
                    "elapsedSeconds": round(elapsed, 3),
                    "rssBytes": rss,
                }
            )
            checkpoint = bridge_soak_report(
                command,
                started_at,
                elapsed,
                samples,
                completed=False,
                process_id=process.pid,
            )
            write_json(SOAK_REPORT, checkpoint)
            if elapsed >= duration_seconds:
                break
            time.sleep(min(sample_seconds, max(0.1, duration_seconds - elapsed)))
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)

    elapsed = time.monotonic() - started
    report = bridge_soak_report(
        command,
        started_at,
        elapsed,
        samples,
        completed=elapsed >= duration_seconds,
        process_id=process.pid,
    )
    write_json(SOAK_REPORT, report)
    return report


def bridge_soak_report(
    command: Sequence[str],
    started_at: datetime,
    elapsed_seconds: float,
    samples: Sequence[Mapping[str, object]],
    *,
    completed: bool,
    process_id: int,
) -> dict[str, object]:
    rss_samples = [int(sample["rssBytes"]) for sample in samples]
    growth = 0 if len(rss_samples) < 2 else rss_samples[-1] - rss_samples[0]
    required_duration_reached = elapsed_seconds >= SOAK_SECONDS
    return {
        "schemaVersion": 1,
        "scenario": "bridge_72_hour_soak",
        "evidenceLevel": "real_process",
        "generatedAt": datetime.now(UTC).isoformat(),
        "revision": git_revision(),
        "startedAt": started_at.isoformat(),
        "commandExecutable": Path(command[0]).name,
        "processId": process_id,
        "completed": completed,
        "passed": completed and required_duration_reached,
        "metrics": {
            "elapsedSeconds": round(elapsed_seconds, 3),
            "requiredSeconds": SOAK_SECONDS,
            "sampleCount": len(samples),
            "maximumRssBytes": max(rss_samples, default=0),
            "rssGrowthBytes": growth,
        },
        "samples": list(samples),
        "releaseGateEligible": completed and required_duration_reached,
    }


def load_report(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CapacityFailure(f"无法读取容量证据 {path}：{error}") from error
    if not isinstance(value, dict):
        raise CapacityFailure(f"容量证据不是 JSON 对象：{path}")
    return value


def evaluate_gate(paths: Iterable[Path]) -> dict[str, object]:
    revision = git_revision()
    reports: dict[str, dict[str, object]] = {}
    for path in paths:
        report = load_report(path)
        scenario = report.get("scenario")
        if not isinstance(scenario, str) or not scenario:
            raise CapacityFailure(f"容量证据缺少 scenario：{path}")
        if scenario in reports:
            raise CapacityFailure(f"容量证据场景重复：{scenario}")
        reports[scenario] = report

    failures: list[str] = []
    for scenario in REQUIRED_REAL_SCENARIOS:
        report = reports.get(scenario)
        if report is None:
            failures.append(f"缺少场景 {scenario}")
            continue
        if report.get("revision") != revision:
            failures.append(f"场景 {scenario} 不是当前 Git 修订")
        if report.get("evidenceLevel") in (None, "model_only", "simulated"):
            failures.append(f"场景 {scenario} 没有真实依赖证据")
        if report.get("passed") is not True:
            failures.append(f"场景 {scenario} 未通过")
        if report.get("releaseGateEligible") is not True:
            failures.append(f"场景 {scenario} 不具备发布门资格")

    report: dict[str, object] = {
        "schemaVersion": 1,
        "scenario": "task_39_capacity_gate",
        "generatedAt": datetime.now(UTC).isoformat(),
        "revision": revision,
        "passed": not failures,
        "requiredScenarios": list(REQUIRED_REAL_SCENARIOS),
        "evidence": {
            name: {
                "evidenceLevel": value.get("evidenceLevel"),
                "passed": value.get("passed"),
                "releaseGateEligible": value.get("releaseGateEligible"),
            }
            for name, value in sorted(reports.items())
        },
        "failures": failures,
    }
    write_json(GATE_REPORT, report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)

    model_parser = subparsers.add_parser("model", help="运行不可用于放行的算法容量模型")
    model_parser.add_argument("--seed", type=int, default=39)

    soak_parser = subparsers.add_parser("bridge-soak", help="监控一个真实 Bridge 子进程")
    soak_parser.add_argument("--duration-seconds", type=int, default=SOAK_SECONDS)
    soak_parser.add_argument("--sample-seconds", type=int, default=30)
    soak_parser.add_argument("command", nargs=argparse.REMAINDER)

    gate_parser = subparsers.add_parser("gate", help="汇总并严格校验真实容量证据")
    gate_parser.add_argument("reports", nargs="+", type=Path)

    arguments = parser.parse_args()
    try:
        if arguments.action == "model":
            report = run_model(arguments.seed)
            print(f"容量算法模型完成：{MODEL_REPORT}")
            print(json.dumps(report["metrics"], ensure_ascii=False, indent=2))
            return 0 if report["passed"] is True else 1
        if arguments.action == "bridge-soak":
            command = list(arguments.command)
            if command and command[0] == "--":
                command = command[1:]
            report = run_bridge_soak(
                command, arguments.duration_seconds, arguments.sample_seconds
            )
            print(f"Bridge 常驻报告：{SOAK_REPORT}")
            return 0 if report["passed"] is True else 1

        report = evaluate_gate(arguments.reports)
        print(f"容量放行报告：{GATE_REPORT}")
        for failure in report["failures"]:
            print(f"- {failure}", file=sys.stderr)
        return 0 if report["passed"] is True else 1
    except CapacityFailure as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
