"""生产备份、错误汇总和容量周报的 systemd 调度单元：确定性生成、安装与核验。"""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import ClassVar, Final

from .config import DeploymentConfig
from .render import DeploymentPaths


ROOT: Final = Path(__file__).resolve().parents[2]
PRODUCTION_SCRIPT: Final = ROOT / "tools" / "production.py"
SYSTEM_UNIT_DIRECTORY: Final = Path("/etc/systemd/system")


class BackupScheduleError(RuntimeError):
    """表示备份调度无法安全生成、安装或核验。"""


@dataclass(frozen=True, slots=True)
class BackupScheduleFiles:
    service_name: str
    timer_name: str
    service_content: str
    timer_content: str


@dataclass(frozen=True, slots=True)
class BackupScheduleInstaller:
    config: DeploymentConfig
    paths: DeploymentPaths
    deployment_config: Path

    def render(self) -> BackupScheduleFiles:
        config_path = _existing_config(self.deployment_config)
        python = _python()
        service_name = f"{self.config.project_name}-backup.service"
        timer_name = f"{self.config.project_name}-backup.timer"
        command = " ".join(
            _systemd_argument(value)
            for value in (
                python.as_posix(),
                PRODUCTION_SCRIPT.as_posix(),
                "backup",
                "--config",
                config_path.as_posix(),
                "--state-dir",
                self.paths.state.as_posix(),
            )
        )
        service = f"""[Unit]
Description=Agent Room 一致性生产备份
Documentation=https://github.com/agent-room/agent-room
Requires=docker.service
After=docker.service network-online.target

[Service]
Type=oneshot
ExecStart={command}
WorkingDirectory={_systemd_path(ROOT.as_posix())}
Environment=PYTHONUTF8=1
Environment=PYTHONUNBUFFERED=1
UMask=0077
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
NoNewPrivileges=true
PrivateTmp=true
TimeoutStartSec=4h
"""
        timer = f"""[Unit]
Description=每 {self.config.backup.rpo_minutes} 分钟触发 Agent Room 生产备份
Documentation=https://github.com/rainyflash/agent-room

[Timer]
OnBootSec=1min
OnCalendar=*:0/{self.config.backup.rpo_minutes}
AccuracySec=1us
Persistent=true
Unit={service_name}

[Install]
WantedBy=timers.target
"""
        return BackupScheduleFiles(service_name, timer_name, service, timer)

    def write_generated(self) -> tuple[Path, Path]:
        return _write_generated(self.paths, self.render())

    def install(self) -> BackupScheduleFiles:
        return _install(self.paths, self.render())

    @staticmethod
    def verify(files: BackupScheduleFiles) -> None:
        _verify_enabled(files)


@dataclass(frozen=True, slots=True)
class _OpsIssueSchedule:
    """定时往私有仓库发 issue 的任务（错误汇总、容量周报）。

    只读生产，往 GitHub 发；令牌放在 `token_file`，单元里只有它的路径。子类给动作名、说明和什么时候跑。
    """

    ACTION: ClassVar[str]
    DESCRIPTION: ClassVar[str]
    TIMER_DESCRIPTION: ClassVar[str]
    CALENDAR: ClassVar[str]

    config: DeploymentConfig
    paths: DeploymentPaths
    deployment_config: Path
    repository: str
    token_file: Path

    def render(self) -> BackupScheduleFiles:
        config_path = _existing_config(self.deployment_config)
        python = _python()
        service_name = f"{self.config.project_name}-{self.ACTION}.service"
        timer_name = f"{self.config.project_name}-{self.ACTION}.timer"
        command = " ".join(
            _systemd_argument(value)
            for value in (
                python.as_posix(),
                PRODUCTION_SCRIPT.as_posix(),
                self.ACTION,
                "--post",
                "--issue-repository",
                self.repository,
                "--issue-token-file",
                self.token_file.as_posix(),
                "--config",
                config_path.as_posix(),
                "--state-dir",
                self.paths.state.as_posix(),
            )
        )
        service = f"""[Unit]
Description={self.DESCRIPTION}
Documentation=https://github.com/rainyflash/agent-room
Requires=docker.service
Wants=network-online.target
After=docker.service network-online.target

[Service]
Type=oneshot
ExecStart={command}
WorkingDirectory={_systemd_path(ROOT.as_posix())}
Environment=PYTHONUTF8=1
Environment=PYTHONUNBUFFERED=1
UMask=0077
Nice=10
NoNewPrivileges=true
PrivateTmp=true
TimeoutStartSec=20min
"""
        # 停机错过的开机后补一次。
        timer = f"""[Unit]
Description={self.TIMER_DESCRIPTION}
Documentation=https://github.com/rainyflash/agent-room

[Timer]
OnCalendar={self.CALENDAR}
Persistent=true
Unit={service_name}

[Install]
WantedBy=timers.target
"""
        return BackupScheduleFiles(service_name, timer_name, service, timer)

    def write_generated(self) -> tuple[Path, Path]:
        return _write_generated(self.paths, self.render())

    def install(self) -> BackupScheduleFiles:
        return _install(self.paths, self.render())

    @staticmethod
    def verify(files: BackupScheduleFiles) -> None:
        _verify_enabled(files)


@dataclass(frozen=True, slots=True)
class ErrorDigestScheduleInstaller(_OpsIssueSchedule):
    """每小时把生产错误汇总发成私有仓库里的 issue（`specs/error-triage/design.md`）。"""

    ACTION: ClassVar[str] = "error-digest"
    DESCRIPTION: ClassVar[str] = "Agent Room 生产错误汇总"
    TIMER_DESCRIPTION: ClassVar[str] = "每小时汇总一次 Agent Room 生产错误"
    # 错开备份（每 15 分钟的整点刻），每小时第 7 分钟跑。
    CALENDAR: ClassVar[str] = "*-*-* *:07:00"


@dataclass(frozen=True, slots=True)
class CapacityReportScheduleInstaller(_OpsIssueSchedule):
    """每周一出一份容量报告，开成私有仓库里的 issue（`specs/measurement/design.md`）。"""

    ACTION: ClassVar[str] = "capacity-report"
    DESCRIPTION: ClassVar[str] = "Agent Room 容量周报"
    TIMER_DESCRIPTION: ClassVar[str] = "每周一出一份 Agent Room 容量周报"
    # 周一北京时间上午九点多；错开备份的整刻和错误汇总的第 7 分钟。
    CALENDAR: ClassVar[str] = "Mon *-*-* 01:23:00"


def _existing_config(path: Path) -> Path:
    config_path = path.expanduser().resolve()
    if not config_path.is_file():
        raise BackupScheduleError(f"部署配置不存在：{config_path}。")
    return config_path


def _python() -> Path:
    python = Path(sys.executable).resolve()
    if not python.is_file():
        raise BackupScheduleError("当前 Python 解释器路径无效。")
    return python


def _write_generated(paths: DeploymentPaths, files: BackupScheduleFiles) -> tuple[Path, Path]:
    directory = paths.generated / "systemd"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    service = directory / files.service_name
    timer = directory / files.timer_name
    _write_unit(service, files.service_content)
    _write_unit(timer, files.timer_content)
    return service, timer


def _install(paths: DeploymentPaths, files: BackupScheduleFiles) -> BackupScheduleFiles:
    if not sys.platform.startswith("linux"):
        raise BackupScheduleError("systemd 调度只能安装在 Linux 生产主机。")
    if not hasattr(os, "geteuid") or os.geteuid() != 0:
        raise BackupScheduleError("安装 systemd 调度必须使用 root 权限。")
    systemctl = shutil.which("systemctl")
    if systemctl is None:
        raise BackupScheduleError("生产主机缺少 systemctl。")
    generated = _write_generated(paths, files)
    _verify_unit_files(generated)
    SYSTEM_UNIT_DIRECTORY.mkdir(mode=0o755, parents=True, exist_ok=True)
    for source in generated:
        destination = SYSTEM_UNIT_DIRECTORY / source.name
        temporary = destination.with_suffix(destination.suffix + ".tmp")
        shutil.copyfile(source, temporary)
        temporary.chmod(0o644)
        os.replace(temporary, destination)
    _systemctl(systemctl, "daemon-reload")
    _systemctl(systemctl, "enable", "--now", files.timer_name)
    _verify_enabled(files)
    return files


def _verify_enabled(files: BackupScheduleFiles) -> None:
    systemctl = shutil.which("systemctl")
    if systemctl is None:
        raise BackupScheduleError("生产主机缺少 systemctl。")
    _systemctl(systemctl, "is-enabled", "--quiet", files.timer_name)
    _systemctl(systemctl, "is-active", "--quiet", files.timer_name)


def _systemd_argument(value: str) -> str:
    if not value or any(character in value for character in "\r\n\0"):
        raise BackupScheduleError("systemd 参数为空或包含控制字符。")
    escaped = value.replace("%", "%%").replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def _systemd_path(value: str) -> str:
    if not value or any(character in value for character in "\r\n\0"):
        raise BackupScheduleError("systemd 路径为空或包含控制字符。")
    if sys.platform.startswith("linux") and not value.startswith("/"):
        raise BackupScheduleError("systemd 工作目录必须是绝对路径。")
    safe = frozenset(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/_.:-")
    encoded: list[str] = []
    for byte in value.encode("utf-8"):
        if byte == ord("%"):
            encoded.append("%%")
        elif byte in safe:
            encoded.append(chr(byte))
        else:
            encoded.append(f"\\x{byte:02x}")
    return "".join(encoded)


def _write_unit(path: Path, content: str) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(content, encoding="utf-8", newline="\n")
    temporary.chmod(0o644)
    os.replace(temporary, path)


def _verify_unit_files(paths: tuple[Path, Path]) -> None:
    executable = shutil.which("systemd-analyze")
    if executable is None:
        raise BackupScheduleError("生产主机缺少 systemd-analyze，无法验证调度单元。")
    result = subprocess.run(
        [executable, "verify", *(str(path) for path in paths)],
        cwd=ROOT,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"退出码 {result.returncode}"
        raise BackupScheduleError(f"systemd 调度单元验证失败：{detail}。")


def _systemctl(executable: str, *arguments: str) -> None:
    result = subprocess.run(
        [executable, *arguments],
        cwd=ROOT,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"退出码 {result.returncode}"
        raise BackupScheduleError(f"systemctl {' '.join(arguments)} 失败：{detail}。")
