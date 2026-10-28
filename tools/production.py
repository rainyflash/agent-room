#!/usr/bin/env python3
"""生成、安装、升级和诊断 Agent Room 生产 Compose。"""

from __future__ import annotations

import argparse
from datetime import UTC, datetime
import json
from pathlib import Path
import sys

from prodops.backup import BackupError
from prodops.config import DeploymentConfig, DeploymentConfigError, load_deployment_config
from prodops.error_digest import (
    LOG_SERVICES as ERROR_DIGEST_SERVICES,
    Digest,
    NoiseTableError,
    load_noise,
    parse_window,
    report as error_report,
    summarize,
)
from prodops.launch_status import report as launch_report
from prodops.ops_issues import (
    DEFAULT_TOKEN_FILE,
    DigestState,
    GitHubIssueTracker,
    OpsIssueError,
    post,
    post_window,
    remember,
)
from prodops.render import DeploymentPaths
from prodops.runtime import ProductionRuntime, ProductionRuntimeError
from prodops.restore import RestoreDrillError, RestoreTarget, parse_target_time
from prodops.schedule import BackupScheduleError, BackupScheduleInstaller, ErrorDigestScheduleInstaller
from prodops.secrets import SecretStoreError


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action",
        choices=(
            "render",
            "validate",
            "preflight",
            "install",
            "upgrade",
            "health",
            "federation",
            "backup",
            "backup-verify",
            "backup-prune",
            "backup-schedule-install",
            "backup-schedule-render",
            "backup-schedule-verify",
            "restore-drill",
            "network-agent-disable",
            "control-plane-reload",
            "registration-apply",
            "telemetry-apply",
            "launch-status",
            "error-digest",
            "error-digest-schedule-install",
            "error-digest-schedule-verify",
            "down",
        ),
    )
    parser.add_argument("--config", type=Path, required=True, help="部署 JSON 配置")
    parser.add_argument("--state-dir", type=Path, required=True, help="持久状态与 Secret 目录")
    parser.add_argument(
        "--full",
        action="store_true",
        help="backup 马上做一份全量；不给时每个 UTC 日第一次做全量，别的时候只打恢复点",
    )
    parser.add_argument("--backup-id", help="要校验或恢复的备份 ID")
    parser.add_argument("--restore-point", help="restore-drill 恢复到这个名字的恢复点")
    parser.add_argument(
        "--target-time",
        help="restore-drill 恢复到这一刻，要带时区，比如 2026-10-11T09:20:00Z",
    )
    parser.add_argument(
        "--since",
        default="1h",
        help="error-digest 从多久以前看起：1h、30m、2d，或带时区的时刻",
    )
    parser.add_argument("--until", help="error-digest 看到哪一刻，要带时区；不给就是现在")
    parser.add_argument(
        "--format",
        choices=("markdown", "json"),
        default="markdown",
        help="error-digest 的输出：给人看的 markdown，或者给下一步用的 json",
    )
    parser.add_argument(
        "--post",
        action="store_true",
        help="error-digest 接着上一次读到的地方汇总，发到 --issue-repository 的 issue 里",
    )
    parser.add_argument("--issue-repository", help="error-digest --post 开 issue 的私有仓库，比如 owner/repo")
    parser.add_argument(
        "--issue-token-file",
        type=Path,
        default=DEFAULT_TOKEN_FILE,
        help="发 issue 用的 GitHub 令牌文件（root、0600），由账号本人放上去",
    )
    parser.add_argument(
        "--network-agent",
        help="要停用的网络 Agent：网络 Agent ID、它的 Agent ID，或还在用的名字",
    )
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    try:
        config = load_deployment_config(arguments.config)
        paths = DeploymentPaths.from_state(arguments.state_dir)
        runtime = ProductionRuntime(config, paths)
        match arguments.action:
            case "render":
                runtime.prepare(generate_signing_key=False)
            case "validate":
                runtime.prepare(generate_signing_key=False)
                runtime.validate_compose()
            case "preflight":
                report = runtime.preflight(require_linux=True, require_dns=True, require_ports=True)
                for warning in report.warnings:
                    print(f"警告：{warning}")
            case "install":
                runtime.install()
            case "upgrade":
                runtime.upgrade()
            case "health":
                runtime.health(timeout_seconds=30)
            case "federation":
                runtime.federation(timeout_seconds=15)
            case "backup":
                run = runtime.scheduled_backup(full=arguments.full)
                if run.full is not None:
                    print(f"备份已原子发布：{run.full.backup_id}")
                if run.point is not None:
                    print(f"恢复点已归档：{run.point.name}")
            case "backup-verify":
                if not arguments.backup_id:
                    raise ValueError("backup-verify 必须提供 --backup-id。")
                manifest = runtime.verify_backup(arguments.backup_id)
                print(f"备份完整性验证通过：{manifest.backup_id}")
            case "backup-prune":
                removed = runtime.prune_backups()
                print(f"已清理 {len(removed)} 个过期备份。")
            case "backup-schedule-render":
                installer = BackupScheduleInstaller(config, paths, arguments.config)
                service, timer = installer.write_generated()
                print(f"备份调度已生成：{service}；{timer}")
            case "backup-schedule-install":
                runtime.prepare(generate_signing_key=True)
                runtime.validate_compose()
                files = BackupScheduleInstaller(config, paths, arguments.config).install()
                print(f"备份调度已启用：{files.timer_name}")
            case "backup-schedule-verify":
                installer = BackupScheduleInstaller(config, paths, arguments.config)
                installer.verify(installer.render())
                print("备份调度已启用且正在运行。")
            case "restore-drill":
                # 三样都不给就恢复到最近一个核对过的恢复点。
                target_time = arguments.target_time
                report = runtime.restore_drill(
                    RestoreTarget(
                        backup_id=arguments.backup_id,
                        restore_point=arguments.restore_point,
                        time=None if target_time is None else parse_target_time(target_time),
                    )
                )
                stop = report.database.target_time or report.database.restore_point_name
                print(
                    f"隔离恢复演练通过：{report.backup_id}，恢复到 {stop}，"
                    f"耗时 {report.duration_seconds:.3f} 秒。"
                )
            case "network-agent-disable":
                if not arguments.network_agent:
                    raise ValueError("network-agent-disable 必须提供 --network-agent。")
                disabled = runtime.disable_network_agent(arguments.network_agent)
                if not disabled:
                    raise ValueError("没有找到还在用的这个网络 Agent。")
                for line in disabled:
                    print(f"已停用网络 Agent：{line}。令牌已作废，约一分钟内离开所有房间。")
            case "control-plane-reload":
                runtime.reload_control_plane()
                print("控制面已按新的配置重建。")
            case "registration-apply":
                runtime.apply_registration()
                state = "开放" if config.identity.registration.is_open else "关闭"
                print(f"Keycloak 注册已{state}。")
            case "telemetry-apply":
                runtime.apply_telemetry()
                if not config.telemetry_enabled:
                    print("监控已关，监控容器已删（数据卷留着），控制面已重建。")
                elif config.alerts_to_ops_issues:
                    print("监控已开，告警由每小时的错误汇总开成 ops 的 issue；控制面已重建。")
                else:
                    print("监控已开，告警发到接收端；控制面已重建。")
            case "launch-status":
                for line in launch_report(runtime.launch_status(), config.network_agents.limits):
                    print(line)
            case "error-digest":
                if arguments.post:
                    _post_error_digest(arguments, paths, runtime)
                else:
                    _print_error_digest(arguments, runtime)
            case "error-digest-schedule-install":
                files = _error_digest_schedule(arguments, config, paths).install()
                print(f"错误汇总调度已启用：{files.timer_name}")
            case "error-digest-schedule-verify":
                installer = _error_digest_schedule(arguments, config, paths)
                installer.verify(installer.render())
                print("错误汇总调度已启用且正在运行。")
            case "down":
                runtime.down()
            case _:
                raise AssertionError("argparse 已约束 action")
    except (
        BackupError,
        BackupScheduleError,
        DeploymentConfigError,
        NoiseTableError,
        OpsIssueError,
        ProductionRuntimeError,
        RestoreDrillError,
        SecretStoreError,
        ValueError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


def _read_error_digest(runtime: ProductionRuntime, since: datetime, until: datetime) -> Digest:
    logs = {service: runtime.service_logs(service, since, until) for service in ERROR_DIGEST_SERVICES}
    # 告警交给 ops 的 issue 时，此刻正在响的告警也汇总进来；告警发到接收端时不重复报。
    if runtime.config.alerts_to_ops_issues:
        logs["alerts"] = runtime.firing_alerts()
    return summarize(logs, load_noise())


def _print_error_digest(arguments: argparse.Namespace, runtime: ProductionRuntime) -> None:
    """只读：读这段时间的容器日志，打码、去掉噪音表里的，打出汇总，不往外发。"""
    since, until = parse_window(arguments.since, arguments.until, datetime.now(UTC))
    digest = _read_error_digest(runtime, since, until)
    window = (since.strftime("%Y-%m-%dT%H:%M:%SZ"), until.strftime("%Y-%m-%dT%H:%M:%SZ"))
    if arguments.format == "json":
        payload = {"since": window[0], "until": window[1],
                   "version": runtime.release_version(), **digest.to_json()}
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        return
    for line in error_report(digest, since=window[0], until=window[1], version=runtime.release_version()):
        print(line)


def _post_error_digest(
    arguments: argparse.Namespace, paths: DeploymentPaths, runtime: ProductionRuntime
) -> None:
    """接着上一次读到的地方汇总，发到私有仓库。先把发现存进状态文件再去 GitHub，GitHub 不通时下次补发。"""
    if not arguments.issue_repository:
        raise ValueError("error-digest --post 必须提供 --issue-repository。")
    state_path = paths.state / "error-digest" / "state.json"
    state = DigestState.load(state_path)
    now = datetime.now(UTC)
    since, until = post_window(state, now)
    remember(state, _read_error_digest(runtime, since, until), until)
    state.save(state_path)
    try:
        tracker = GitHubIssueTracker.from_token_file(arguments.issue_repository, arguments.issue_token_file)
        result = post(state, tracker, version=runtime.release_version(), now=now)
    finally:
        state.save(state_path)
    print(
        f"错误汇总 {since:%Y-%m-%dT%H:%M:%SZ} – {until:%Y-%m-%dT%H:%M:%SZ}：开了 {len(result.opened)} 个，"
        f"重新打开 {len(result.reopened)} 个，补了 {len(result.commented)} 条每天的次数，"
        f"已有的记了 {result.counted} 种，超出上限 {result.overflow} 种。"
    )


def _error_digest_schedule(
    arguments: argparse.Namespace, config: DeploymentConfig, paths: DeploymentPaths
) -> ErrorDigestScheduleInstaller:
    if not arguments.issue_repository:
        raise ValueError("错误汇总调度必须提供 --issue-repository。")
    return ErrorDigestScheduleInstaller(
        config, paths, arguments.config, arguments.issue_repository, arguments.issue_token_file
    )


if __name__ == "__main__":
    raise SystemExit(main())
