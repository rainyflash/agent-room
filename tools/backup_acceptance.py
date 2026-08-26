#!/usr/bin/env python3
"""在临时 Docker 拓扑中执行真实备份与隔离恢复验收。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile

from prodops.config import DeploymentConfig
from prodops.render import DeploymentPaths
from prodops.runtime import ProductionRuntime


ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, help="成功后写入的脱敏 JSON 证据")
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    if shutil.which("docker") is None:
        print("缺少 Docker，无法执行真实备份恢复验收。", file=sys.stderr)
        return 1
    with tempfile.TemporaryDirectory(prefix="agent-room-backup-acceptance-") as temporary:
        root = Path(temporary)
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["projectName"] = f"agent-room-backup-{secrets.token_hex(4)}"
        value["backup"]["repository"] = (root / "backups").as_posix()
        value["telemetry"]["enabled"] = False
        config = DeploymentConfig.from_mapping(value)
        paths = DeploymentPaths.from_state(root / "state")
        runtime = ProductionRuntime(config, paths)
        runtime.prepare(generate_signing_key=True)
        use_native_postgres_volume(paths.worker_override)
        runtime.validate_compose()
        runtime.prepare_backup_repository()
        compose = runtime.compose_command()
        try:
            run([*compose, "build", "migrate", "identity"])
            run([*compose, "up", "--detach", "--wait", "postgres"])
            run([*compose, "up", "--detach", "object-store"])
            runtime.initialize_object_store()
            run([*compose, "run", "--rm", "migrate"])
            run([*compose, "up", "--detach", "--wait", "synapse", "identity"])
            upload_fixture(compose)
            manifest = runtime.backup()
            report = runtime.restore_drill(manifest.backup_id)
            if report.object_count < 1:
                raise RuntimeError("恢复演练没有验证测试对象。")
            evidence = {
                "schemaVersion": 1,
                "backupId": report.backup_id,
                "rpoTargetMinutes": report.rpo_target_minutes,
                "rtoTargetSeconds": report.rto_target_seconds,
                "durationSeconds": round(report.duration_seconds, 3),
                "rtoMet": report.rto_met,
                "objectCount": report.object_count,
                "database": {
                    "replayReachedTarget": report.database.replay_reached_target,
                    "logicalArchivesVerified": report.database.logical_archives_verified,
                    "databasesVerified": list(report.database.databases_verified),
                    "projectionMemberships": report.database.projection_memberships,
                    "projectionRooms": report.database.projection_rooms,
                },
            }
            if arguments.evidence is not None:
                write_evidence(arguments.evidence, evidence)
            print(json.dumps(evidence, ensure_ascii=False, indent=2, sort_keys=True))
        finally:
            run([*compose, "down", "--volumes", "--remove-orphans"], check=False)
    return 0


def upload_fixture(compose: list[str]) -> None:
    script = """
set -eu
access=$(cat /run/secrets/s3_access_key)
secret=$(cat /run/secrets/s3_secret_key)
mc --config-dir /tmp/mc alias set source "$AGENT_ROOM_CONTENT_S3_ENDPOINT" "$access" "$secret" --api S3v4 >/dev/null
printf 'agent-room-backup-acceptance' | mc --config-dir /tmp/mc pipe "source/$AGENT_ROOM_CONTENT_S3_BUCKET/acceptance/object.txt" >/dev/null
""".strip()
    run(
        [
            *compose,
            "run",
            "--rm",
            "--no-deps",
            "--entrypoint",
            "/bin/sh",
            "object-backup",
            "-ec",
            script,
        ]
    )


def use_native_postgres_volume(override_path: Path) -> None:
    """物理备份验收避开 Windows 宿主文件共享层，使用 Docker 原生卷。"""
    override = json.loads(override_path.read_text(encoding="utf-8"))
    services = override.setdefault("services", {})
    if not isinstance(services, dict):
        raise RuntimeError("Compose 验收覆盖文件的 services 结构无效。")
    services["postgres"] = {
        "volumes": ["acceptance-postgres:/var/lib/postgresql"],
    }
    override["volumes"] = {"acceptance-postgres": {}}
    write_evidence(override_path, override)


def run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if check and result.returncode != 0:
        raise RuntimeError(f"验收命令失败（{command[0]}），退出码 {result.returncode}。")
    return result


def write_evidence(path: Path, value: object) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    temporary.replace(path)


if __name__ == "__main__":
    raise SystemExit(main())
