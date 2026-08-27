"""生产备份 systemd 调度单元的确定性生成、安装与核验。"""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Final

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
        config_path = self.deployment_config.expanduser().resolve()
        if not config_path.is_file():
            raise BackupScheduleError(f"部署配置不存在：{config_path}。")
        python = Path(sys.executable).resolve()
        if not python.is_file():
            raise BackupScheduleError("当前 Python 解释器路径无效。")
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
        files = self.render()
        directory = self.paths.generated / "systemd"
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        service = directory / files.service_name
        timer = directory / files.timer_name
        _write_unit(service, files.service_content)
        _write_unit(timer, files.timer_content)
        return service, timer

    def install(self) -> BackupScheduleFiles:
        if not sys.platform.startswith("linux"):
            raise BackupScheduleError("systemd 备份调度只能安装在 Linux 生产主机。")
        if not hasattr(os, "geteuid") or os.geteuid() != 0:
            raise BackupScheduleError("安装 systemd 备份调度必须使用 root 权限。")
        systemctl = shutil.which("systemctl")
        if systemctl is None:
            raise BackupScheduleError("生产主机缺少 systemctl。")
        files = self.render()
        generated = self.write_generated()
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
        self.verify(files)
        return files

    @staticmethod
    def verify(files: BackupScheduleFiles) -> None:
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
