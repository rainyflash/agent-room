#!/usr/bin/env python3
"""用生产同款 PostgreSQL 镜像实跑物理备份脚本，再按恢复演练的做法还原、起库。

单元测试碰不到真实的 pg_basebackup、pg_verifybackup 和 pg_waldump。改备份脚本或恢复代码时，
这里在一次性 Docker 环境里走一遍：开着 WAL 归档的库 → postgres-base-backup.sh →
materialize_base_backup → 带恢复点的 PITR 起库，确认到达恢复点、数据一条不少。只在 Linux 上跑。
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time

from prodops.restore import materialize_base_backup


ROOT = Path(__file__).resolve().parents[1]
IMAGE = "postgres:18.6-alpine"
SCRIPT = ROOT / "infra" / "production" / "postgres-base-backup.sh"
BACKUP_ID = "20261009T090000000000Z-0123abcd"
USER = "agent_room_bootstrap"
ROWS_BEFORE_BACKUP = 200_500


class DrillFailure(RuntimeError):
    pass


def docker(*arguments: str, capture: bool = False) -> str:
    result = subprocess.run(
        ["docker", *arguments], capture_output=capture, text=True, encoding="utf-8", check=False
    )
    if result.returncode != 0:
        detail = (result.stderr or "").strip()[-2000:] if capture else ""
        raise DrillFailure(f"docker {arguments[0]} 失败（{result.returncode}）：{detail}")
    return result.stdout if capture else ""


def wait_until(description: str, check, timeout_seconds: int) -> None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(1)
    raise DrillFailure(f"{timeout_seconds} 秒内没等到{description}。")


def succeeds(*arguments: str) -> bool:
    return subprocess.run(["docker", *arguments], capture_output=True, check=False).returncode == 0


def sql(container: str, database: str, statement: str, *, socket: str | None = None) -> str:
    host = ["-h", socket] if socket else []
    return docker(
        "exec", "-u", "postgres", container, "psql", *host, "-U", USER, "-d", database,
        "-tAc", statement, capture=True,
    ).strip()


def try_sql(container: str, database: str, statement: str, *, socket: str | None = None) -> str | None:
    host = ["-h", socket] if socket else []
    result = subprocess.run(
        ["docker", "exec", "-u", "postgres", container, "psql", *host, "-U", USER, "-d", database,
         "-tAc", statement],
        capture_output=True, text=True, encoding="utf-8", check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def start_source(name: str, repository: Path, password: Path) -> str:
    container = f"{name}-source"
    docker(
        "run", "-d", "--name", container,
        "-e", f"POSTGRES_USER={USER}", "-e", "POSTGRES_PASSWORD_FILE=/run/secrets/password",
        "-v", f"{password}:/run/secrets/password:ro",
        "-v", f"{repository / 'wal'}:/archive",
        IMAGE,
        # 与生产 compose 的归档设置一致。
        "-c", "wal_level=replica", "-c", "archive_mode=on", "-c", "archive_timeout=60s",
        "-c", "archive_command=test ! -f /archive/%f && cp %p /archive/%f",
        capture=True,
    )
    # 镜像初始化时先起一个临时实例，要等初始化完、正式实例起来以后再连。
    wait_until(
        "源数据库就绪",
        lambda: "init process complete" in docker("logs", container, capture=True) + _stderr_logs(container)
        and succeeds("exec", container, "pg_isready", "-h", "127.0.0.1", "-U", USER),
        120,
    )
    sql(container, "postgres", "CREATE DATABASE agent_room")
    sql(container, "agent_room",
        "CREATE TABLE messages AS SELECT g AS id, md5(g::text) AS body FROM generate_series(1, 200000) g")
    sql(container, "agent_room", "INSERT INTO messages SELECT g, 'later' FROM generate_series(200001, 200500) g")
    return container


def _stderr_logs(container: str) -> str:
    result = subprocess.run(["docker", "logs", container], capture_output=True, text=True,
                            encoding="utf-8", check=False)
    return result.stderr


def run_backup(source: str, repository: Path, password: Path) -> None:
    docker(
        "run", "--rm", "--network", f"container:{source}", "--user", "0:0",
        "-v", f"{repository}:/backup", "-v", f"{repository / 'wal'}:/archive:ro",
        "-v", f"{password}:/run/secrets/postgres_bootstrap_password:ro",
        "-v", f"{SCRIPT}:/scripts/postgres-base-backup.sh:ro",
        "-e", f"AGENT_ROOM_BACKUP_ID={BACKUP_ID}", "-e", "AGENT_ROOM_DB_HOST=127.0.0.1",
        "-e", "AGENT_ROOM_DB_PORT=5432", "-e", "AGENT_ROOM_DB_TLS_MODE=disable",
        "--entrypoint", "/bin/sh", IMAGE, "/scripts/postgres-base-backup.sh",
    )


def hand_back(path: Path) -> None:
    """容器以 root 写下的文件交还给当前用户，宿主上的 Python 才读得到、删得掉。"""

    docker("run", "--rm", "-v", f"{path}:/work", "--entrypoint", "/bin/sh", IMAGE,
           "-c", f"chown -R {os.getuid()}:{os.getgid()} /work")


def restore(name: str, physical: Path, work: Path) -> dict[str, object]:
    point = json.loads((physical / "restore-point.json").read_text(encoding="utf-8"))
    work.mkdir(mode=0o700)
    data = work / "data"
    materialize_base_backup(physical / "base", data)
    shutil.copytree(physical / "wal", work / "wal")
    (data / "recovery.signal").touch(mode=0o600)
    with (data / "postgresql.auto.conf").open("a", encoding="utf-8", newline="\n") as stream:
        stream.write("restore_command = 'cp /wal/%f %p'\n")
        stream.write(f"recovery_target_name = '{point['name']}'\n")
        stream.write("recovery_target_action = 'promote'\n")

    # 照 runtime.restore_database：拷进卷、交给 postgres 用户，再以只读 WAL 起库。
    for volume in (f"{name}-data", f"{name}-wal"):
        docker("volume", "create", volume, capture=True)
    docker(
        "run", "--rm", "--network", "none", "--user", "0:0", "--entrypoint", "/bin/sh",
        "-v", f"{data}:/source-data:ro", "-v", f"{work / 'wal'}:/source-wal:ro",
        "-v", f"{name}-data:/target-data", "-v", f"{name}-wal:/target-wal", IMAGE, "-c",
        "set -eu; cp -a /source-data/. /target-data/; cp -a /source-wal/. /target-wal/; "
        "chown -R 70:70 /target-data /target-wal; chmod 0700 /target-data /target-wal",
    )
    restored = f"{name}-restored"
    docker(
        "run", "-d", "--name", restored, "--network", "none", "--user", "70:70",
        "--entrypoint", "postgres", "-e", "PGDATA=/var/lib/postgresql/data",
        "-v", f"{name}-data:/var/lib/postgresql/data", "-v", f"{name}-wal:/wal:ro",
        "--tmpfs", "/tmp:rw,noexec,nosuid,size=128m,uid=70,gid=70",
        IMAGE, "-D", "/var/lib/postgresql/data", "-c", "listen_addresses=",
        "-c", "unix_socket_directories=/tmp",
        capture=True,
    )
    wait_until(
        "恢复的库提升为主库",
        lambda: try_sql(restored, "postgres", "SELECT NOT pg_is_in_recovery()", socket="/tmp") == "t",
        180,
    )
    replayed = sql(
        restored, "postgres",
        f"SELECT coalesce(pg_last_wal_replay_lsn() >= '{point['lsn']}'::pg_lsn, false)", socket="/tmp",
    )
    logs = docker("logs", restored, capture=True) + _stderr_logs(restored)
    reached = replayed == "t" or f'restore point "{point["name"]}"' in logs
    rows = int(sql(restored, "agent_room", "SELECT count(*) FROM messages", socket="/tmp"))
    if not reached:
        raise DrillFailure("恢复的库没有到达备份脚本建的恢复点。")
    if rows != ROWS_BEFORE_BACKUP:
        raise DrillFailure(f"恢复后 messages 有 {rows} 行，应为 {ROWS_BEFORE_BACKUP}。")
    return {"restorePoint": point["name"], "reachedRestorePoint": True, "rows": rows}


def drill(name: str, root: Path) -> dict[str, object]:
    repository = root / "backups"
    (repository / "wal").mkdir(parents=True)
    password = root / "password"
    password.write_text(secrets.token_urlsafe(24), encoding="utf-8")
    password.chmod(0o644)
    # 源库以 postgres 用户（uid 70）往归档目录写 WAL。
    docker("run", "--rm", "-v", f"{repository / 'wal'}:/archive", "--entrypoint", "/bin/sh", IMAGE,
           "-c", "chown 70:70 /archive && chmod 0700 /archive")
    source = start_source(name, repository, password)
    run_backup(source, repository, password)
    hand_back(root)
    physical = repository / f".partial-{BACKUP_ID}" / "postgres"
    files = sorted(
        (path.relative_to(physical).as_posix(), path.stat().st_size)
        for path in physical.rglob("*")
        if path.is_file()
    )
    names = {path for path, _ in files}
    for required in ("base/base.tar.gz", "base/pg_wal.tar.gz", "base/backup_manifest", "restore-point.json"):
        if required not in names:
            raise DrillFailure(f"备份缺少 {required}。")
    if (physical / "base" / "PG_VERSION").exists():
        raise DrillFailure("备份里还留着未压缩的数据目录。")
    evidence: dict[str, object] = {"files": dict(files)}
    evidence.update(restore(name, physical, root / "restore"))
    return evidence


def cleanup(name: str) -> None:
    for container in (f"{name}-source", f"{name}-restored"):
        subprocess.run(["docker", "rm", "-f", container], capture_output=True, check=False)
    for volume in (f"{name}-data", f"{name}-wal"):
        subprocess.run(["docker", "volume", "rm", "-f", volume], capture_output=True, check=False)


def main() -> int:
    if not sys.platform.startswith("linux") or shutil.which("docker") is None:
        print("需要 Linux 上的 Docker。", file=sys.stderr)
        return 1
    name = f"agent-room-backup-e2e-{secrets.token_hex(4)}"
    with tempfile.TemporaryDirectory(prefix="agent-room-backup-e2e-") as temporary:
        root = Path(temporary)
        try:
            evidence = drill(name, root)
        except DrillFailure as error:
            for container in (f"{name}-source", f"{name}-restored"):
                subprocess.run(["docker", "logs", "--tail", "40", container], check=False)
            print(f"物理备份实跑失败：{error}", file=sys.stderr)
            return 1
        finally:
            cleanup(name)
            hand_back(root)
    print(json.dumps(evidence, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
