#!/usr/bin/env python3
"""用生产同款 SeaweedFS 和 rclone 镜像实跑对象增量备份，再照恢复演练的做法按清单取回。

对象备份脚本只在生产的定时备份里真跑，单元测试碰不到真实的 rclone。改对象备份或恢复代码时，
这里在一次性 Docker 环境里走一遍：存几个对象 → object-backup.sh → 删一个、改一个、加一个 →
再跑一次 → 没变的不重新下载，被删、被改的旧版本挪进当天的目录 → 两套快照都能照各自的清单
一个不少地取回。只在 Linux 上跑。
"""

from __future__ import annotations

from datetime import UTC, date, datetime
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time

from prodops.restore import RestoreDrillError, read_object_inventory, restore_mirrored_objects


ROOT = Path(__file__).resolve().parents[1]
SEAWEEDFS = "ghcr.io/chrislusf/seaweedfs:4.44"
RCLONE = "rclone/rclone:1.75.1"
SCRIPT = ROOT / "infra" / "production" / "object-backup.sh"
BUCKET = "agent-room-content"
FIRST = "20261011T090000000000Z-0123abcd"
SECOND = "20261011T091500000000Z-0123abce"
BEFORE = {"content/01": b"first", "content/02": b"second", "attachments/03": b"third"}
AFTER = {"content/02": b"second, edited", "attachments/03": b"third", "content/04": b"fourth"}
# 和 object-backup.sh 一样只从环境变量读 rclone 配置。
RCLONE_ENVIRONMENT = """
mkdir -p /tmp/rclone
export HOME=/tmp/rclone XDG_CACHE_HOME=/tmp/rclone/cache RCLONE_CONFIG=/tmp/rclone/rclone.conf
export RCLONE_CONFIG_SOURCE_TYPE=s3 RCLONE_CONFIG_SOURCE_PROVIDER=SeaweedFS
export RCLONE_CONFIG_SOURCE_ENDPOINT=http://127.0.0.1:8333
RCLONE_CONFIG_SOURCE_ACCESS_KEY_ID=$(cat /run/secrets/s3_access_key)
RCLONE_CONFIG_SOURCE_SECRET_ACCESS_KEY=$(cat /run/secrets/s3_secret_key)
export RCLONE_CONFIG_SOURCE_ACCESS_KEY_ID RCLONE_CONFIG_SOURCE_SECRET_ACCESS_KEY
"""


class DrillFailure(RuntimeError):
    pass


def docker(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["docker", *arguments],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )


def rclone(store: str, secrets_directory: Path, *arguments: str, script: str) -> subprocess.CompletedProcess[str]:
    """照 compose 的 object-backup 起 rclone 容器：宿主用户、根目录只读、密钥按文件挂进去。"""

    return docker(
        "run",
        "--rm",
        "--network",
        f"container:{store}",
        "--user",
        f"{os.getuid()}:{os.getgid()}",
        "--read-only",
        "--tmpfs",
        "/tmp:rw,noexec,nosuid,size=64m",
        "-v",
        f"{secrets_directory / 's3_access_key'}:/run/secrets/s3_access_key:ro",
        "-v",
        f"{secrets_directory / 's3_secret_key'}:/run/secrets/s3_secret_key:ro",
        *arguments,
        "--entrypoint",
        "/bin/sh",
        RCLONE,
        "-ec",
        script,
    )


def start_store(name: str, root: Path) -> tuple[str, Path]:
    secrets_directory = root / "secrets"
    secrets_directory.mkdir()
    access_key, secret_key = secrets.token_hex(10), secrets.token_urlsafe(30)
    (secrets_directory / "s3_access_key").write_text(access_key, encoding="utf-8")
    (secrets_directory / "s3_secret_key").write_text(secret_key, encoding="utf-8")
    config = root / "seaweedfs"
    config.mkdir()
    # 照 render.py 生成的 s3.json：只有控制面那一个身份。
    identity = {
        "name": "agent-room-control-plane",
        "credentials": [{"accessKey": access_key, "secretKey": secret_key}],
        "actions": ["Admin", "Read", "Write", "List", "Tagging"],
    }
    (config / "s3.json").write_text(json.dumps({"identities": [identity]}), encoding="utf-8")
    for path in (*secrets_directory.iterdir(), config / "s3.json"):
        path.chmod(0o644)
    store = f"{name}-store"
    started = docker(
        "run",
        "-d",
        "--name",
        store,
        "-v",
        f"{config}:/config:ro",
        "-v",
        f"{name}-data:/data",
        SEAWEEDFS,
        "server",
        "-s3",
        "-dir=/data",
        "-ip.bind=0.0.0.0",
        "-s3.config=/config/s3.json",
    )
    if started.returncode != 0:
        raise DrillFailure(f"对象存储起不来：{started.stderr.strip()[-2000:]}")
    # 能建桶、能写能删才算起来了：卷服务器比 S3 端口晚就绪。
    probe = (
        f"{RCLONE_ENVIRONMENT}\nexport RCLONE_RETRIES=1 RCLONE_LOW_LEVEL_RETRIES=1\n"
        f"rclone mkdir 'source:{BUCKET}'\n"
        f"printf ready | rclone rcat 'source:{BUCKET}/ready'\n"
        f"rclone deletefile 'source:{BUCKET}/ready'"
    )
    deadline = time.monotonic() + 120
    while True:
        ready = rclone(store, secrets_directory, script=probe)
        if ready.returncode == 0:
            return store, secrets_directory
        if time.monotonic() > deadline:
            raise DrillFailure(f"对象存储 120 秒内没准备好：{ready.stderr.strip()[-2000:]}")
        time.sleep(2)


def change_objects(
    store: str, secrets_directory: Path, written: dict[str, bytes], deleted: tuple[str, ...] = ()
) -> None:
    lines = [RCLONE_ENVIRONMENT]
    lines.extend(f"rclone deletefile 'source:{BUCKET}/{key}'" for key in deleted)
    lines.extend(
        f"printf '%s' '{content.decode()}' | rclone rcat 'source:{BUCKET}/{key}'"
        for key, content in written.items()
    )
    result = rclone(store, secrets_directory, script="\n".join(lines))
    if result.returncode != 0:
        raise DrillFailure(f"改对象失败：{result.stderr.strip()[-2000:]}")


def run_backup(store: str, secrets_directory: Path, repository: Path, backup_id: str) -> Path:
    """跑生产的 object-backup.sh，返回这套快照的清单。"""

    result = rclone(
        store,
        secrets_directory,
        "-v",
        f"{repository}:/backup",
        "-v",
        f"{SCRIPT}:/scripts/object-backup.sh:ro",
        "-e",
        f"AGENT_ROOM_BACKUP_ID={backup_id}",
        "-e",
        "AGENT_ROOM_CONTENT_S3_ENDPOINT=http://127.0.0.1:8333",
        "-e",
        f"AGENT_ROOM_CONTENT_S3_BUCKET={BUCKET}",
        script="exec /bin/sh /scripts/object-backup.sh",
    )
    if result.returncode != 0:
        raise DrillFailure(f"对象备份失败：{result.stderr.strip()[-2000:]}")
    snapshot = repository / f".partial-{backup_id}" / "objects"
    if (snapshot / "data").exists():
        raise DrillFailure("快照里还在整份复制对象。")
    return snapshot / "source-inventory.ndjson"


def files(directory: Path) -> dict[str, bytes]:
    if not directory.is_dir():
        return {}
    return {
        path.relative_to(directory).as_posix(): path.read_bytes()
        for path in directory.rglob("*")
        if path.is_file()
    }


def expect(label: str, actual: object, expected: object) -> None:
    if actual != expected:
        raise DrillFailure(f"{label}：实际 {actual!r}，应为 {expected!r}。")


def drill(name: str, root: Path) -> dict[str, object]:
    repository = root / "backups"
    repository.mkdir()
    objects = repository / "objects"
    mirror = objects / "mirror"
    store, secrets_directory = start_store(name, root)
    started_on = datetime.now(UTC).date()

    change_objects(store, secrets_directory, BEFORE)
    first = run_backup(store, secrets_directory, repository, FIRST)
    expect("第一次同步后的镜像", files(mirror), BEFORE)
    expect("第一套快照的清单", sorted(entry.path for entry in read_object_inventory(first)), sorted(BEFORE))
    unchanged = (mirror / "attachments" / "03").stat().st_ino

    change_objects(
        store,
        secrets_directory,
        {key: AFTER[key] for key in ("content/02", "content/04")},
        ("content/01",),
    )
    second = run_backup(store, secrets_directory, repository, SECOND)
    expect("第二次同步后的镜像", files(mirror), AFTER)
    expect("没变的对象重新下载了", (mirror / "attachments" / "03").stat().st_ino, unchanged)
    removed = {
        f"{day.name}/{key}": content
        for day in sorted((objects / "removed").iterdir())
        for key, content in files(day).items()
    }
    expect("挪走的旧版本", sorted(removed.values()), [b"first", b"second"])
    for key in removed:
        if date.fromisoformat(key.split("/", 1)[0]) < started_on:
            raise DrillFailure(f"旧版本挪进的目录日期不对：{sorted(removed)}。")

    evidence: dict[str, object] = {"removed": sorted(removed)}
    for backup_id, inventory, expected in ((FIRST, first, BEFORE), (SECOND, second, AFTER)):
        target = root / f"restore-{backup_id}"
        count, total = restore_mirrored_objects(objects, inventory, started_on, target)
        expect(f"{backup_id} 取回的对象", files(target), expected)
        evidence[backup_id] = {"objects": count, "bytes": total}
    return evidence


def cleanup(name: str) -> None:
    docker("rm", "-f", f"{name}-store")
    docker("volume", "rm", "-f", f"{name}-data")


def main() -> int:
    if not sys.platform.startswith("linux") or shutil.which("docker") is None:
        print("需要 Linux 上的 Docker。", file=sys.stderr)
        return 1
    name = f"agent-room-object-e2e-{secrets.token_hex(4)}"
    with tempfile.TemporaryDirectory(prefix="agent-room-object-e2e-") as temporary:
        try:
            evidence = drill(name, Path(temporary))
        except (DrillFailure, RestoreDrillError) as error:
            logs = docker("logs", "--tail", "40", f"{name}-store")
            print(logs.stdout + logs.stderr, file=sys.stderr)
            print(f"对象增量备份实跑失败：{error}", file=sys.stderr)
            return 1
        finally:
            cleanup(name)
    print(json.dumps(evidence, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
