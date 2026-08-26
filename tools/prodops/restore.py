"""隔离恢复演练的证据模型与协调器。"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
from typing import Callable, Protocol

from .backup import BackupError, BackupManifest, BackupRepository
from .config import DeploymentConfig
from .render import DeploymentPaths


RESTORE_POINT_NAME = re.compile(r"^[A-Za-z0-9_]{1,200}$")


class RestoreDrillError(RuntimeError):
    """表示隔离恢复没有达到可用性或完整性门禁。"""


@dataclass(frozen=True, slots=True)
class DatabaseRestoreEvidence:
    restore_point_name: str
    restore_point_lsn: str
    replay_reached_target: bool
    logical_archives_verified: int
    databases_verified: tuple[str, ...]
    projection_memberships: int
    projection_rooms: int
    deletion_ledger_entries: int
    deletion_replays_queued: int


class RestoreBackend(Protocol):
    def restore_database(
        self,
        backup_directory: Path,
        drill_directory: Path,
        restore_point_name: str,
        restore_point_lsn: str,
        account_deletion_ledger: Path,
    ) -> DatabaseRestoreEvidence:
        """在隔离运行时恢复数据库并重建派生投影。"""


@dataclass(frozen=True, slots=True)
class RestoreDrillReport:
    schema_version: int
    backup_id: str
    started_at: str
    completed_at: str
    duration_seconds: float
    rpo_target_minutes: int
    rto_target_seconds: int
    rto_met: bool
    signing_key_sha256: str
    object_count: int
    object_bytes: int
    database: DatabaseRestoreEvidence

    def to_mapping(self) -> dict[str, object]:
        return {
            "schemaVersion": self.schema_version,
            "backupId": self.backup_id,
            "startedAt": self.started_at,
            "completedAt": self.completed_at,
            "durationSeconds": round(self.duration_seconds, 3),
            "rpoTargetMinutes": self.rpo_target_minutes,
            "rtoTargetSeconds": self.rto_target_seconds,
            "rtoMet": self.rto_met,
            "signingKeySha256": self.signing_key_sha256,
            "objectCount": self.object_count,
            "objectBytes": self.object_bytes,
            "database": {
                "restorePointName": self.database.restore_point_name,
                "restorePointLsn": self.database.restore_point_lsn,
                "replayReachedTarget": self.database.replay_reached_target,
                "logicalArchivesVerified": self.database.logical_archives_verified,
                "databasesVerified": list(self.database.databases_verified),
                "projectionMemberships": self.database.projection_memberships,
                "projectionRooms": self.database.projection_rooms,
                "deletionLedgerEntries": self.database.deletion_ledger_entries,
                "deletionReplaysQueued": self.database.deletion_replays_queued,
            },
        }


@dataclass(slots=True)
class RestoreDrillCoordinator:
    config: DeploymentConfig
    paths: DeploymentPaths
    repository: BackupRepository
    backend: RestoreBackend
    clock: Callable[[], datetime] = lambda: datetime.now(UTC)

    def run(self, backup_id: str) -> RestoreDrillReport:
        started = self.clock().astimezone(UTC)
        manifest = self.repository.verify(backup_id)
        if manifest.database_mode != "embedded" or self.config.database.mode != "embedded":
            raise RestoreDrillError("外部 PostgreSQL 必须由供应商隔离恢复流程执行，不能伪装成本地 PITR。")
        if manifest.server_name != self.config.public.server_name:
            raise RestoreDrillError("备份所属 server_name 与当前部署不一致。")
        backup_directory = self.repository.root / backup_id
        drill_directory = self._create_drill_directory(backup_id, started)
        try:
            restore_name, restore_lsn = _read_restore_point(
                backup_directory / "postgres" / "restore-point.json"
            )
            signing_digest = self._restore_identity(backup_directory, drill_directory, manifest)
            object_count, object_bytes = self._restore_objects(
                backup_directory, drill_directory, manifest
            )
            database = self.backend.restore_database(
                backup_directory,
                drill_directory,
                restore_name,
                restore_lsn,
                self._stage_account_deletion_ledger(drill_directory),
            )
            completed = self.clock().astimezone(UTC)
            duration = max(0.0, (completed - started).total_seconds())
            report = RestoreDrillReport(
                schema_version=1,
                backup_id=backup_id,
                started_at=_utc(started),
                completed_at=_utc(completed),
                duration_seconds=duration,
                rpo_target_minutes=self.config.backup.rpo_minutes,
                rto_target_seconds=4 * 60 * 60,
                rto_met=duration <= 4 * 60 * 60,
                signing_key_sha256=signing_digest,
                object_count=object_count,
                object_bytes=object_bytes,
                database=database,
            )
            if not report.rto_met or not database.replay_reached_target:
                raise RestoreDrillError("恢复演练未达到 RTO 或 PITR 恢复点门禁。")
            _write_json(drill_directory / "report.json", report.to_mapping())
            return report
        except BaseException:
            _write_failure_marker(drill_directory)
            raise

    def _stage_account_deletion_ledger(self, drill_directory: Path) -> Path:
        ledger = self.repository.load_account_deletion_ledger()
        target = drill_directory / "privacy" / "account-deletions.json"
        target.parent.mkdir(mode=0o700)
        _write_json(target, ledger.to_mapping())
        return target

    def _create_drill_directory(self, backup_id: str, started: datetime) -> Path:
        root = self.paths.state / "restore-drills"
        root.mkdir(mode=0o700, parents=True, exist_ok=True)
        suffix = started.strftime("%Y%m%dT%H%M%SZ")
        path = root / f"{backup_id}-{suffix}"
        try:
            path.mkdir(mode=0o700)
        except FileExistsError as error:
            raise RestoreDrillError("同一备份与时间的恢复演练目录已存在。") from error
        return path

    @staticmethod
    def _restore_identity(
        backup_directory: Path,
        drill_directory: Path,
        manifest: BackupManifest,
    ) -> str:
        source = backup_directory / "identity" / "synapse.signing.key"
        target = drill_directory / "identity" / "synapse.signing.key"
        target.parent.mkdir(mode=0o700)
        shutil.copyfile(source, target)
        expected = _artifact_digest(manifest, "identity/synapse.signing.key")
        actual = _sha256(target)
        if actual != expected:
            raise RestoreDrillError("恢复后的 Synapse signing key 摘要不一致。")
        return actual

    @staticmethod
    def _restore_objects(
        backup_directory: Path,
        drill_directory: Path,
        manifest: BackupManifest,
    ) -> tuple[int, int]:
        source = backup_directory / "objects" / "data"
        target = drill_directory / "objects"
        if source.is_dir():
            shutil.copytree(source, target)
        else:
            target.mkdir(mode=0o700)
        count = 0
        total = 0
        prefix = "objects/data/"
        for artifact in manifest.artifacts:
            if not artifact.path.startswith(prefix):
                continue
            relative = artifact.path.removeprefix(prefix)
            restored = target / Path(relative)
            if not restored.is_file() or _sha256(restored) != artifact.sha256:
                raise RestoreDrillError(f"恢复对象摘要不一致：{relative}。")
            count += 1
            total += artifact.byte_length
        return count, total


def _read_restore_point(path: Path) -> tuple[str, str]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RestoreDrillError("恢复点元数据缺失或损坏。") from error
    if not isinstance(value, dict) or set(value) != {"name", "lsn", "lastRequiredWal"}:
        raise RestoreDrillError("恢复点元数据字段无效。")
    name = value.get("name")
    lsn = value.get("lsn")
    wal = value.get("lastRequiredWal")
    if not isinstance(name, str) or not RESTORE_POINT_NAME.fullmatch(name):
        raise RestoreDrillError("恢复点名称无效。")
    if not isinstance(lsn, str) or not re.fullmatch(r"[0-9A-F]+/[0-9A-F]+", lsn):
        raise RestoreDrillError("恢复点 LSN 无效。")
    if not isinstance(wal, str) or not re.fullmatch(r"[0-9A-F]{24}", wal):
        raise RestoreDrillError("恢复点 WAL 文件名无效。")
    return name, lsn


def _artifact_digest(manifest: BackupManifest, path: str) -> str:
    for artifact in manifest.artifacts:
        if artifact.path == path:
            return artifact.sha256
    raise BackupError(f"备份清单缺少工件：{path}。")


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _utc(value: datetime) -> str:
    return value.isoformat().replace("+00:00", "Z")


def _write_json(path: Path, value: object) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    temporary.chmod(0o600)
    os.replace(temporary, path)


def _write_failure_marker(path: Path) -> None:
    try:
        (path / "FAILED").write_text("恢复演练失败；本目录不得作为恢复证据。\n", encoding="utf-8")
    except OSError:
        pass
