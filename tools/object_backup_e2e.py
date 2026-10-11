#!/usr/bin/env python3
"""用生产同款 SeaweedFS 和 rclone 镜像实跑对象增量备份，再照恢复演练的做法按清单取回。

对象备份脚本只在生产的定时备份里真跑，单元测试碰不到真实的 rclone。改对象备份或恢复代码时，
这里在一次性 Docker 环境里走一遍：存几个对象 → object-backup.sh 做全量 → 删一个、改一个、加一个 →
只打恢复点的那次只同步、不写快照 → 再做一次全量 → 没变的不重新下载，被删、被改的旧版本挪进当天的
目录 → 两套快照都能照各自的清单一个不少地取回。只在 Linux 上跑。
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
POINT = "20261011T091500000000Z-0123abce"
SECOND = "20261011T093000000000Z-0123abcf"
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

# 照 render.py 生成的 s3.json，只有控制面那一个身份。钥匙在容器里随机生成，和 s3.json 一起写进卷。
KEYS = r"""
umask 022
access=$(head -c 10 /dev/urandom | od -An -tx1 | tr -d ' \n')
secret=$(head -c 30 /dev/urandom | od -An -tx1 | tr -d ' \n')
printf '%s' "$access" >/keys/s3_access_key
printf '%s' "$secret" >/keys/s3_secret_key
printf '{"identities":[{"name":"agent-room-control-plane","credentials":[{"accessKey":"%s","secretKey":"%s"}],' \
  "$access" "$secret" >/keys/s3.json
printf '"actions":["Admin","Read","Write","List","Tagging"]}]}' >>/keys/s3.json
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


def rclone(store: str, keys: str, *arguments: str, script: str) -> subprocess.CompletedProcess[str]:
    """照 compose 的 object-backup 起 rclone 容器：宿主用户、根目录只读、钥匙按文件挂在 /run/secrets。"""

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
        f"{keys}:/run/secrets:ro",
        *arguments,
        "--entrypoint",
        "/bin/sh",
        RCLONE,
        "-ec",
        script,
    )


def start_store(name: str) -> tuple[str, str]:
    """起一个一次性的对象存储。访问钥匙在容器里随机生成、写进 Docker 卷，测试本身不经手。"""

    keys = f"{name}-keys"
    generated = docker("run", "--rm", "-v", f"{keys}:/keys", "--entrypoint", "/bin/sh", RCLONE, "-ec", KEYS)
    if generated.returncode != 0:
        raise DrillFailure(f"生成对象存储的访问钥匙失败：{generated.stderr.strip()[-2000:]}")
    store = f"{name}-store"
    started = docker(
        "run",
        "-d",
        "--name",
        store,
        "-v",
        f"{keys}:/config:ro",
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
        ready = rclone(store, keys, script=probe)
        if ready.returncode == 0:
            return store, keys
        if time.monotonic() > deadline:
            raise DrillFailure(f"对象存储 120 秒内没准备好：{ready.stderr.strip()[-2000:]}")
        time.sleep(2)


def change_objects(store: str, keys: str, written: dict[str, bytes], deleted: tuple[str, ...] = ()) -> None:
    lines = [RCLONE_ENVIRONMENT]
    lines.extend(f"rclone deletefile 'source:{BUCKET}/{key}'" for key in deleted)
    lines.extend(
        f"printf '%s' '{content.decode()}' | rclone rcat 'source:{BUCKET}/{key}'"
        for key, content in written.items()
    )
    result = rclone(store, keys, script="\n".join(lines))
    if result.returncode != 0:
        raise DrillFailure(f"改对象失败：{result.stderr.strip()[-2000:]}")


def run_backup(store: str, keys: str, repository: Path, backup_id: str, kind: str = "full") -> Path:
    """跑生产的 object-backup.sh，返回这套快照的清单；只打恢复点的那几次（point）不写快照。"""

    result = rclone(
        store,
        keys,
        "-v",
        f"{repository}:/backup",
        "-v",
        f"{SCRIPT}:/scripts/object-backup.sh:ro",
        "-e",
        f"AGENT_ROOM_BACKUP_ID={backup_id}",
        "-e",
        f"AGENT_ROOM_BACKUP_KIND={kind}",
        "-e",
        "AGENT_ROOM_CONTENT_S3_ENDPOINT=http://127.0.0.1:8333",
        "-e",
        f"AGENT_ROOM_CONTENT_S3_BUCKET={BUCKET}",
        script="exec /bin/sh /scripts/object-backup.sh",
    )
    if result.returncode != 0:
        raise DrillFailure(f"对象备份失败：{result.stderr.strip()[-2000:]}")
    if kind == "point" and (repository / f".partial-{backup_id}").exists():
        raise DrillFailure("只打恢复点的那次也写了快照。")
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
    store, keys = start_store(name)
    started_on = datetime.now(UTC).date()

    change_objects(store, keys, BEFORE)
    first = run_backup(store, keys, repository, FIRST)
    expect("第一次同步后的镜像", files(mirror), BEFORE)
    expect("第一套快照的清单", sorted(entry.path for entry in read_object_inventory(first)), sorted(BEFORE))
    unchanged = (mirror / "attachments" / "03").stat().st_ino

    change_objects(store, keys, {key: AFTER[key] for key in ("content/02", "content/04")}, ("content/01",))
    run_backup(store, keys, repository, POINT, "point")
    expect("只打恢复点那次同步后的镜像", files(mirror), AFTER)
    second = run_backup(store, keys, repository, SECOND)
    expect("第二次全量后的镜像", files(mirror), AFTER)
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
    docker("volume", "rm", "-f", f"{name}-data", f"{name}-keys")


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
