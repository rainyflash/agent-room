#!/usr/bin/env python3
"""校验 Agent Room 观测契约，并在生产副本上执行可恢复故障演练。"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import sys
import time
from typing import Final
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

if __package__:
    from tools.prodops.config import (
        DeploymentConfig,
        DeploymentConfigError,
        load_deployment_config,
    )
    from tools.prodops.render import DeploymentPaths
    from tools.prodops.runtime import ProductionRuntime, ProductionRuntimeError
else:
    from prodops.config import DeploymentConfig, DeploymentConfigError, load_deployment_config
    from prodops.render import DeploymentPaths
    from prodops.runtime import ProductionRuntime, ProductionRuntimeError


ROOT: Final = Path(__file__).resolve().parent.parent
PRODUCTION: Final = ROOT / "infra" / "production"
RULES: Final = PRODUCTION / "observability-rules.yaml"
RULE_TESTS: Final = PRODUCTION / "observability-rules.test.yaml"
DASHBOARD: Final = PRODUCTION / "grafana" / "dashboards" / "agent-room-overview.json"
REPORT: Final = ROOT / "artifacts" / "observability" / "fault-drill-report.json"
PROMETHEUS_IMAGE: Final = "prom/prometheus:v3.13.1"
EXPECTED_ALERTS: Final = frozenset(
    {
        "AgentRoomApiAvailabilityFastBurn",
        "AgentRoomApiLatencyHigh",
        "AgentRoomDependencyUnavailable",
        "AgentRoomOutboxBacklog",
        "AgentRoomOutboxDeadLetters",
        "AgentRoomProjectionStalled",
        "AgentRoomCoreEndpointUnavailable",
        "AgentRoomFederationEndpointUnavailable",
        "AgentRoomBridgeAvailabilityLow",
        "AgentRoomPostgresExporterDown",
        "AgentRoomBackupRpoBreached",
        "AgentRoomRestoreDrillStale",
        "AgentRoomRestoreRtoExceeded",
    }
)
BANNED_LABEL_FRAGMENTS: Final = (
    "user",
    "principal",
    "agent",
    "room",
    "event",
    "message",
    "token",
    "path",
    "url",
    "filename",
    "digest",
)


class ObservabilityError(RuntimeError):
    """表示观测配置或故障演练没有满足硬性验收条件。"""


@dataclass(frozen=True, slots=True)
class DrillTarget:
    name: str
    service: str | None
    alert_name: str
    instance: str | None


@dataclass(frozen=True, slots=True)
class DrillEvidence:
    target: str
    alert_name: str
    detected_seconds: float
    recovered_seconds: float
    synthetic: bool


DRILL_TARGETS: Final = {
    "control-plane": DrillTarget(
        "control-plane", "control-plane", "AgentRoomCoreEndpointUnavailable", "control-plane"
    ),
    "matrix": DrillTarget("matrix", "synapse", "AgentRoomCoreEndpointUnavailable", "matrix"),
    "object-store": DrillTarget(
        "object-store", "object-store", "AgentRoomCoreEndpointUnavailable", "object-store"
    ),
    "oidc": DrillTarget("oidc", "identity", "AgentRoomCoreEndpointUnavailable", "oidc"),
    "federation": DrillTarget(
        "federation", "gateway", "AgentRoomFederationEndpointUnavailable", None
    ),
    "bridge": DrillTarget("bridge", None, "AgentRoomBridgeAvailabilityLow", None),
}


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    subparsers.add_parser("validate", help="校验规则、告警模拟、仪表盘与标签隐私契约")
    drill = subparsers.add_parser("drill", help="停止服务并验证告警检测与恢复")
    drill.add_argument("--config", type=Path, required=True)
    drill.add_argument("--state-dir", type=Path, required=True)
    drill.add_argument(
        "--target",
        choices=(*DRILL_TARGETS, "all"),
        default="all",
    )
    drill.add_argument("--timeout-seconds", type=int, default=720)
    drill.add_argument(
        "--confirm-stop-services",
        action="store_true",
        help="确认允许脚本逐个停止并恢复生产副本服务",
    )
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    try:
        validate_contract()
        if arguments.action == "drill":
            if not arguments.confirm_stop_services:
                raise ObservabilityError("故障演练必须显式提供 --confirm-stop-services。")
            if not 120 <= arguments.timeout_seconds <= 1_800:
                raise ObservabilityError("故障演练超时必须在 120–1800 秒之间。")
            config = load_deployment_config(arguments.config)
            paths = DeploymentPaths.from_state(arguments.state_dir)
            evidence = run_drill(
                config,
                paths,
                target=arguments.target,
                timeout_seconds=arguments.timeout_seconds,
            )
            write_report(evidence)
            print(f"故障演练通过，证据已写入：{REPORT}")
        else:
            print("观测契约、13 个分页告警模拟与仪表盘查询均通过。")
    except (
        DeploymentConfigError,
        ObservabilityError,
        ProductionRuntimeError,
        ValueError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


def validate_contract() -> None:
    rules = RULES.read_text(encoding="utf-8")
    dashboard = json.loads(DASHBOARD.read_text(encoding="utf-8"))
    if not isinstance(dashboard, dict) or dashboard.get("uid") != "agent-room-service-truth":
        raise ObservabilityError("Grafana 仪表盘缺少稳定 UID。")

    alerts = frozenset(re.findall(r"^\s*- alert: ([A-Za-z][A-Za-z0-9]+)$", rules, re.MULTILINE))
    if alerts != EXPECTED_ALERTS:
        missing = ", ".join(sorted(EXPECTED_ALERTS - alerts)) or "无"
        extra = ", ".join(sorted(alerts - EXPECTED_ALERTS)) or "无"
        raise ObservabilityError(f"分页告警集合漂移；缺少：{missing}；额外：{extra}。")
    for alert in alerts:
        block = _alert_block(rules, alert)
        for required in ("impact:", "diagnostic:", "runbook_url:"):
            if required not in block:
                raise ObservabilityError(f"{alert} 缺少 {required.removesuffix(':')} 注解。")

    _validate_low_cardinality_labels(rules)
    expressions = _dashboard_expressions(dashboard)
    for required in (
        "agent_room:api_availability:ratio_30d",
        "agent_room:api_latency:p95_5m",
        "agent_room:api_error_budget_remaining:ratio_30d",
        "agent_room_backup_last_success_timestamp_seconds",
        "agent_room_restore_drill_duration_seconds",
    ):
        if not any(required in expression for expression in expressions):
            raise ObservabilityError(f"仪表盘没有展示必需事实：{required}。")
    _promtool("check", "rules", "/work/observability-rules.yaml")
    _promtool("test", "rules", "observability-rules.test.yaml", workdir="/work")


def run_drill(
    config: DeploymentConfig,
    paths: DeploymentPaths,
    *,
    target: str,
    timeout_seconds: int,
) -> tuple[DrillEvidence, ...]:
    if not config.telemetry_enabled:
        raise ObservabilityError("部署未启用 telemetry，不能执行故障演练。")
    runtime = ProductionRuntime(config, paths)
    runtime.prepare(generate_signing_key=False)
    runtime.validate_compose()
    selected = tuple(DRILL_TARGETS.values()) if target == "all" else (DRILL_TARGETS[target],)
    evidence: list[DrillEvidence] = []
    for drill_target in selected:
        if drill_target.name == "object-store" and config.object_store.mode != "embedded":
            raise ObservabilityError("外部对象存储不能由本工具停止；请在供应商沙箱执行等价演练。")
        if drill_target.service is None:
            _promtool("test", "rules", "observability-rules.test.yaml", workdir="/work")
            evidence.append(DrillEvidence(drill_target.name, drill_target.alert_name, 0.0, 0.0, True))
            continue
        evidence.append(_drill_service(runtime, drill_target, timeout_seconds))
    return tuple(evidence)


def _drill_service(
    runtime: ProductionRuntime,
    target: DrillTarget,
    timeout_seconds: int,
) -> DrillEvidence:
    assert target.service is not None
    command = runtime.compose_command()
    started = time.monotonic()
    _run([*command, "stop", "--timeout", "30", target.service])
    try:
        _wait_alert(target, firing=True, timeout_seconds=timeout_seconds)
        detected = time.monotonic() - started
    finally:
        _run([*command, "up", "--detach", "--wait", target.service])
    recovered_started = time.monotonic()
    _wait_alert(target, firing=False, timeout_seconds=timeout_seconds)
    return DrillEvidence(
        target=target.name,
        alert_name=target.alert_name,
        detected_seconds=round(detected, 3),
        recovered_seconds=round(time.monotonic() - recovered_started, 3),
        synthetic=False,
    )


def _wait_alert(target: DrillTarget, *, firing: bool, timeout_seconds: int) -> None:
    deadline = time.monotonic() + timeout_seconds
    selector = f'ALERTS{{alertname="{target.alert_name}",alertstate="firing"'
    if target.instance is not None:
        selector += f',instance="{target.instance}"'
    selector += "}"
    last_error = "尚未查询"
    while time.monotonic() < deadline:
        try:
            count = _query_prometheus_count(selector)
            if (count > 0) == firing:
                return
            last_error = f"当前告警序列数为 {count}"
        except ObservabilityError as error:
            last_error = str(error)
        time.sleep(5)
    expected = "触发" if firing else "恢复"
    raise ObservabilityError(f"{target.name} 告警未在时限内{expected}：{last_error}。")


def _query_prometheus_count(expression: str) -> int:
    url = f"http://127.0.0.1:9090/api/v1/query?query={quote(f'count({expression})')}"
    request = Request(url, headers={"User-Agent": "agent-room-observability-drill/1"})
    try:
        with urlopen(request, timeout=5) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (HTTPError, URLError, TimeoutError, OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ObservabilityError("无法查询本机 Prometheus。") from error
    if not isinstance(payload, dict) or payload.get("status") != "success":
        raise ObservabilityError("Prometheus 查询没有返回成功状态。")
    data = payload.get("data")
    if not isinstance(data, dict):
        raise ObservabilityError("Prometheus 查询缺少 data。")
    result = data.get("result")
    if not isinstance(result, list) or not result:
        return 0
    sample = result[0]
    if not isinstance(sample, dict):
        raise ObservabilityError("Prometheus 样本形状无效。")
    value = sample.get("value")
    if not isinstance(value, list) or len(value) != 2 or not isinstance(value[1], str):
        raise ObservabilityError("Prometheus 样本值无效。")
    return int(float(value[1]))


def _alert_block(rules: str, alert: str) -> str:
    match = re.search(
        rf"^\s*- alert: {re.escape(alert)}$([\s\S]*?)(?=^\s*- alert: |\Z)",
        rules,
        re.MULTILINE,
    )
    if match is None:
        raise ObservabilityError(f"找不到告警规则：{alert}。")
    return match.group(0)


def _validate_low_cardinality_labels(rules: str) -> None:
    label_names = set(re.findall(r"([A-Za-z_][A-Za-z0-9_]*)\s*(?:=~|!~|!=|=)\s*\"", rules))
    for label in label_names:
        normalized = label.lower()
        if any(fragment in normalized for fragment in BANNED_LABEL_FRAGMENTS):
            raise ObservabilityError(f"规则包含禁止的高基数或敏感标签：{label}。")


def _dashboard_expressions(dashboard: dict[str, object]) -> tuple[str, ...]:
    panels = dashboard.get("panels")
    if not isinstance(panels, list):
        raise ObservabilityError("Grafana 仪表盘缺少 panels。")
    expressions: list[str] = []
    for panel in panels:
        if not isinstance(panel, dict):
            raise ObservabilityError("Grafana panel 必须是对象。")
        targets = panel.get("targets", [])
        if not isinstance(targets, list):
            raise ObservabilityError("Grafana targets 必须是数组。")
        for target in targets:
            if isinstance(target, dict) and isinstance(target.get("expr"), str):
                expressions.append(target["expr"])
    return tuple(expressions)


def _promtool(*arguments: str, workdir: str | None = None) -> None:
    command = [
        "docker",
        "run",
        "--rm",
        "--entrypoint",
        "/bin/promtool",
        "--volume",
        f"{PRODUCTION.resolve().as_posix()}:/work:ro",
    ]
    if workdir is not None:
        command.extend(("--workdir", workdir))
    command.extend((PROMETHEUS_IMAGE, *arguments))
    _run(command)


def _run(command: list[str]) -> None:
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"退出码 {result.returncode}"
        raise ObservabilityError(f"命令失败（{command[0]}）：{detail[-3000:]}")


def write_report(evidence: tuple[DrillEvidence, ...]) -> None:
    REPORT.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    payload = {
        "schemaVersion": 1,
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "targets": [asdict(item) for item in evidence],
    }
    REPORT.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
