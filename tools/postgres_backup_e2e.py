#!/usr/bin/env python3
"""用生产同款 PostgreSQL 镜像实跑物理备份和 WAL 归档脚本，再按恢复演练的做法还原、起库。

单元测试碰不到真实的 pg_basebackup、pg_verifybackup 和 pg_waldump。改备份脚本或恢复代码时，
这里在一次性 Docker 环境里走一遍：照生产 compose 的设置开着 WAL 归档的库 →
postgres-base-backup.sh → postgres-wal-archive.sh 接着收两次 → 收到坏段时停下、什么都不删 →
materialize_base_backup → 按全量自己的恢复点、按之后一直留着的 WAL 的恢复点各起一次库，
确认到达恢复点、数据一条不少。只在 Linux 上跑。
"""

from __future__ import annotations

import gzip
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time

from prodops.restore import RESTORE_COMMAND, materialize_base_backup
from prodops.restore_point import RestorePoint
from prodops.wal_store import WalAnchor, WalStore


ROOT = Path(__file__).resolve().parents[1]
IMAGE = "postgres:18.6-alpine"
COMPOSE = ROOT / "infra" / "production" / "compose.yaml"
SCRIPT = ROOT / "infra" / "production" / "postgres-base-backup.sh"
ARCHIVE_SCRIPT = ROOT / "infra" / "production" / "postgres-wal-archive.sh"
BACKUP_ID = "20261009T090000000000Z-0123abcd"
POINT_IDS = (
    "20261009T091500000000Z-0123abc1",
    "20261009T093000000000Z-0123abc2",
    "20261009T094500000000Z-0123abc3",
)
USER = "agent_room_bootstrap"
SEGMENT_BYTES = 16 * 1024 * 1024
ROWS_BEFORE_BACKUP = 200_500
ROWS_BEFORE_SECOND_POINT = 201_500


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


def production_settings() -> list[str]:
    """生产 compose 里 postgres 的 -c 设置，归档超时缩到 60 秒；hba_file 要挂文件，这里不用。"""

    lines = COMPOSE.read_text(encoding="utf-8").splitlines()
    command = lines.index("    command:", lines.index("  postgres:"))
    values: list[str] = []
    for line in lines[command + 1:]:
        if line.strip().startswith("#"):
            continue
        if not line.startswith("      - "):
            break
        values.append(line.removeprefix("      - "))
    settings = [
        value.replace("${AGENT_ROOM_BACKUP_ARCHIVE_TIMEOUT_SECONDS:-900}", "60").replace("$$", "$")
        for flag, value in zip(values, values[1:])
        if flag == "-c" and not value.startswith("hba_file=")
    ]
    names = {setting.split("=", 1)[0] for setting in settings}
    if not {"archive_command", "wal_compression", "checkpoint_timeout"} <= names:
        raise DrillFailure(f"没从 compose 读到生产的归档设置：{sorted(names)}。")
    return settings


def start_source(name: str, repository: Path, password: Path) -> str:
    container = f"{name}-source"
    options = [part for setting in production_settings() for part in ("-c", setting)]
    docker(
        "run", "-d", "--name", container,
        "-e", f"POSTGRES_USER={USER}", "-e", "POSTGRES_PASSWORD_FILE=/run/secrets/password",
        "-v", f"{password}:/run/secrets/password:ro",
        "-v", f"{repository / 'wal'}:/archive",
        IMAGE, *options,
        capture=True,
    )
    # 镜像初始化时先起一个临时实例，要等初始化完、正式实例起来以后再连。
    wait_until(
        "源数据库就绪",
        lambda: "init process complete" in docker("logs", container, capture=True) + _stderr_logs(container)
        and succeeds("exec", container, "pg_isready", "-h", "127.0.0.1", "-U", USER),
        120,
    )
    for setting, expected in (("wal_compression", "zstd"), ("checkpoint_timeout", "15min")):
        actual = sql(container, "postgres", f"SHOW {setting}")
        if actual != expected:
            raise DrillFailure(f"源数据库的 {setting} 是 {actual}，应为 {expected}。")
    sql(container, "postgres", "CREATE DATABASE agent_room")
    sql(container, "agent_room",
        "CREATE TABLE messages AS SELECT g AS id, md5(g::text) AS body FROM generate_series(1, 200000) g")
    sql(container, "agent_room", "INSERT INTO messages SELECT g, 'later' FROM generate_series(200001, 200500) g")
    return container


def _stderr_logs(container: str) -> str:
    result = subprocess.run(["docker", "logs", container], capture_output=True, text=True,
                            encoding="utf-8", check=False)
    return result.stderr


def database_environment() -> list[str]:
    return [
        "-e", "AGENT_ROOM_DB_HOST=127.0.0.1", "-e", "AGENT_ROOM_DB_PORT=5432",
        "-e", "AGENT_ROOM_DB_TLS_MODE=disable",
    ]


def run_backup(source: str, repository: Path, password: Path) -> None:
    docker(
        "run", "--rm", "--network", f"container:{source}", "--user", "0:0",
        "-v", f"{repository}:/backup", "-v", f"{repository / 'wal'}:/archive:ro",
        "-v", f"{password}:/run/secrets/postgres_bootstrap_password:ro",
        "-v", f"{SCRIPT}:/scripts/postgres-base-backup.sh:ro",
        "-e", f"AGENT_ROOM_BACKUP_ID={BACKUP_ID}", *database_environment(),
        "--entrypoint", "/bin/sh", IMAGE, "/scripts/postgres-base-backup.sh",
    )


def run_archive(
    source: str, repository: Path, password: Path, point_id: str, anchor: WalAnchor
) -> subprocess.CompletedProcess[str]:
    """照 compose 的 postgres-wal-archive：以 root 跑，只挂整个仓库，收好的文件交给当前用户。"""

    return subprocess.run(
        [
            "docker", "run", "--rm", "--network", f"container:{source}", "--user", "0:0",
            "-v", f"{repository}:/backup",
            "-v", f"{password}:/run/secrets/postgres_bootstrap_password:ro",
            "-v", f"{ARCHIVE_SCRIPT}:/scripts/postgres-wal-archive.sh:ro",
            "-e", f"AGENT_ROOM_BACKUP_ID={point_id}",
            "-e", f"AGENT_ROOM_WAL_ANCHOR={anchor.environment_value}",
            "-e", f"AGENT_ROOM_HOST_UID={os.getuid()}", "-e", f"AGENT_ROOM_HOST_GID={os.getgid()}",
            *database_environment(),
            "--entrypoint", "/bin/sh", IMAGE, "/scripts/postgres-wal-archive.sh",
        ],
        capture_output=True, text=True, encoding="utf-8", check=False,
    )


def archive_with_root(repository: Path, command: str) -> str:
    """归档目录归 PostgreSQL（uid 70）所有，看和改都用 root 容器。"""

    return docker(
        "run", "--rm", "-v", f"{repository / 'wal'}:/archive", "--entrypoint", "/bin/sh", IMAGE,
        "-c", command, capture=True,
    )


def archived_segments(repository: Path) -> list[str]:
    names = archive_with_root(repository, "ls /archive").split()
    return sorted(name for name in names if len(name) == 24)


def hand_back(path: Path) -> None:
    """容器以 root 写下的文件交还给当前用户，宿主上的 Python 才读得到、删得掉。"""

    docker("run", "--rm", "-v", f"{path}:/work", "--entrypoint", "/bin/sh", IMAGE,
           "-c", f"chown -R {os.getuid()}:{os.getgid()} /work")


def check_store(store: WalStore, kinds: list[str]) -> None:
    points = store.points()
    if [point.kind for point in points] != kinds:
        raise DrillFailure(f"恢复点记录是 {[point.kind for point in points]}，应为 {kinds}。")
    for segment in store.segments():
        packed = store.root / f"{segment}.gz"
        digest = (store.root / f"{segment}.gz.sha256").read_text(encoding="utf-8").split()
        if digest != [hashlib.sha256(packed.read_bytes()).hexdigest(), f"{segment}.gz"]:
            raise DrillFailure(f"{segment} 的 SHA-256 记录对不上。")
        with gzip.open(packed) as stream:
            if len(stream.read()) != SEGMENT_BYTES:
                raise DrillFailure(f"{segment} 解压后不是整段。")
    last = points[-1].segment
    if any(name <= last for name in archived_segments(store.root.parent)):
        raise DrillFailure("收好的段还留在归档目录里。")


def archive_point(source: str, repository: Path, password: Path, index: int, anchor: WalAnchor) -> None:
    result = run_archive(source, repository, password, POINT_IDS[index], anchor)
    if result.returncode != 0:
        raise DrillFailure(f"WAL 归档失败：{result.stderr.strip()[-2000:]}")


def broken_segment_is_refused(source: str, repository: Path, password: Path, anchor: WalAnchor) -> None:
    """收到坏段时停下：不收、不记、不删归档目录里的原文件。"""

    store = WalStore(repository / "wal-store")
    before = store.points()
    sql(source, "agent_room", "INSERT INTO messages SELECT g, 'gap' FROM generate_series(300001, 302000) g")
    sql(source, "postgres", "SELECT pg_switch_wal()")
    wait_until("坏段之前的段归档完", lambda: len(archived_segments(repository)) >= 1, 60)
    damaged = archived_segments(repository)[0]
    # 第二页中间写进一段乱码：段的大小不变，只有逐条读才发现得了。
    archive_with_root(
        repository,
        f"printf 'not-a-wal-record-not-a-wal-record' | dd of=/archive/{damaged} bs=1 seek=16448 conv=notrunc",
    )
    result = run_archive(source, repository, password, POINT_IDS[2], anchor)
    if result.returncode == 0 or "读不通" not in result.stderr:
        raise DrillFailure(f"坏段没有被拦下：{result.returncode} {result.stderr.strip()[-2000:]}")
    if store.points() != before or damaged not in archived_segments(repository):
        raise DrillFailure("坏段被拦下以后，恢复点记录或归档目录变了。")
    if (store.root / f"{damaged}.gz").exists():
        raise DrillFailure("坏段被收进了仓库。")


def restore(name: str, physical: Path, wal_sources: list[Path], target: str, lsn: str, expected_rows: int,
            work: Path) -> dict[str, object]:
    work.mkdir(mode=0o700)
    data = work / "data"
    materialize_base_backup(physical / "base", data)
    wal = work / "wal"
    wal.mkdir()
    for source in wal_sources:
        for path in source.iterdir():
            if len(path.name.removesuffix(".gz")) == 24 and not path.name.endswith(".sha256"):
                shutil.copyfile(path, wal / path.name)
    (data / "recovery.signal").touch(mode=0o600)
    with (data / "postgresql.auto.conf").open("a", encoding="utf-8", newline="\n") as stream:
        stream.write(f"restore_command = '{RESTORE_COMMAND}'\n")
        stream.write(f"recovery_target_name = '{target}'\n")
        stream.write("recovery_target_action = 'promote'\n")

    # 照 runtime.restore_database：拷进卷、交给 postgres 用户，再以只读 WAL 起库。
    for volume in (f"{name}-data", f"{name}-wal"):
        docker("volume", "create", volume, capture=True)
    docker(
        "run", "--rm", "--network", "none", "--user", "0:0", "--entrypoint", "/bin/sh",
        "-v", f"{data}:/source-data:ro", "-v", f"{wal}:/source-wal:ro",
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
        f"SELECT coalesce(pg_last_wal_replay_lsn() >= '{lsn}'::pg_lsn, false)", socket="/tmp",
    )
    logs = docker("logs", restored, capture=True) + _stderr_logs(restored)
    reached = replayed == "t" or f'restore point "{target}"' in logs
    rows = int(sql(restored, "agent_room", "SELECT count(*) FROM messages", socket="/tmp"))
    if not reached:
        raise DrillFailure(f"恢复的库没有到达恢复点 {target}。")
    if rows != expected_rows:
        raise DrillFailure(f"恢复到 {target} 后 messages 有 {rows} 行，应为 {expected_rows}。")
    return {"restorePoint": target, "reachedRestorePoint": True, "rows": rows}


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
    # 照 BackupRepository.publish：核对完的全量从临时目录改名成正式目录。归档目录还归源库所有。
    hand_back(repository / f".partial-{BACKUP_ID}")
    os.replace(repository / f".partial-{BACKUP_ID}", repository / BACKUP_ID)
    physical = repository / BACKUP_ID / "postgres"
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

    point = RestorePoint.load(physical / "restore-point.json")
    anchor = WalAnchor(BACKUP_ID, point.name, point.lsn, point.last_required_wal)
    store = WalStore(repository / "wal-store")
    archive_point(source, repository, password, 0, anchor)
    check_store(store, ["anchor", "point"])
    sql(source, "agent_room", "INSERT INTO messages SELECT g, 'second' FROM generate_series(200501, 201500) g")
    archive_point(source, repository, password, 1, anchor)
    check_store(store, ["anchor", "point", "point"])
    second = store.points()[-1]
    sql(source, "agent_room", "INSERT INTO messages SELECT g, 'after' FROM generate_series(201501, 201800) g")
    broken_segment_is_refused(source, repository, password, anchor)

    evidence: dict[str, object] = {"files": dict(files), "walStore": list(store.segments())}
    evidence["fromFull"] = restore(
        f"{name}-full", physical, [physical / "wal"], point.name, point.lsn, ROWS_BEFORE_BACKUP,
        root / "restore-full",
    )
    evidence["fromWalStore"] = restore(
        f"{name}-store", physical, [physical / "wal", store.root], second.name, second.lsn,
        ROWS_BEFORE_SECOND_POINT, root / "restore-store",
    )
    return evidence


def cleanup(name: str) -> None:
    containers = [f"{name}-source", f"{name}-full-restored", f"{name}-store-restored"]
    for container in containers:
        subprocess.run(["docker", "rm", "-f", container], capture_output=True, check=False)
    for restore_name in (f"{name}-full", f"{name}-store"):
        for volume in (f"{restore_name}-data", f"{restore_name}-wal"):
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
            for container in (f"{name}-source", f"{name}-full-restored", f"{name}-store-restored"):
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
