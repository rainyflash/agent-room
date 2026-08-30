"""生产备份清单、原子发布、完整性校验与保留清理。"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import asdict, dataclass
from datetime import UTC, datetime, timedelta
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shutil
from typing import Callable, Final, Iterator, Protocol
from uuid import UUID

from .config import DeploymentConfig
from .render import DeploymentPaths
from .restore_point import RestorePoint, RestorePointError, WAL_SEGMENT


BACKUP_SCHEMA_VERSION: Final = 1
BACKUP_ID: Final = re.compile(r"^[0-9]{8}T[0-9]{12}Z-[0-9a-f]{8}$")
SHA256: Final = re.compile(r"^[0-9a-f]{64}$")
WAL_BACKUP_MARKER: Final = re.compile(r"^([0-9A-F]{24})\.[0-9A-F]{8}\.backup$")
LOCK_STALE_AFTER: Final = timedelta(hours=24)
MINIMUM_BACKUP_HEADROOM_BYTES: Final = 2 * 1024 * 1024 * 1024
MANIFEST_NAME: Final = "manifest.json"
ACCOUNT_DELETION_ARTIFACT: Final = "privacy/account-deletions.json"
ACCOUNT_DELETION_LEDGER_NAME: Final = "ACCOUNT_DELETION_LEDGER.json"


class BackupError(RuntimeError):
    """表示备份不完整、被篡改或无法安全发布。"""


class BackupCapture(Protocol):
    def capture_backup_payload(self, backup_id: str) -> None:
        """把外部依赖快照写入指定的临时备份目录。"""


@dataclass(frozen=True, slots=True, order=True)
class AccountDeletionLedgerEntry:
    job_id: str
    principal_id: str
    matrix_user_id: str
    completed_at: str

    @classmethod
    def from_mapping(cls, value: object) -> "AccountDeletionLedgerEntry":
        root = _mapping(value, "账户删除账本条目")
        if set(root) != {"jobId", "principalId", "matrixUserId", "completedAt"}:
            raise BackupError("账户删除账本条目字段不完整或包含未知字段。")
        job_id = _uuid_text(root.get("jobId"), "jobId", version=7)
        principal_id = _uuid_text(root.get("principalId"), "principalId")
        matrix_user_id = _text(root.get("matrixUserId"), "matrixUserId")
        if not 4 <= len(matrix_user_id) <= 512 or not matrix_user_id.startswith("@"):
            raise BackupError("账户删除账本的 Matrix 用户 ID 无效。")
        completed_at = _text(root.get("completedAt"), "completedAt")
        _parse_utc(completed_at)
        return cls(job_id, principal_id, matrix_user_id, completed_at)

    def to_mapping(self) -> dict[str, str]:
        return {
            "jobId": self.job_id,
            "principalId": self.principal_id,
            "matrixUserId": self.matrix_user_id,
            "completedAt": self.completed_at,
        }


@dataclass(frozen=True, slots=True)
class AccountDeletionLedger:
    entries: tuple[AccountDeletionLedgerEntry, ...]

    @classmethod
    def empty(cls) -> "AccountDeletionLedger":
        return cls(())

    @classmethod
    def from_mapping(cls, value: object) -> "AccountDeletionLedger":
        root = _mapping(value, "账户删除账本")
        if set(root) != {"schemaVersion", "entries"}:
            raise BackupError("账户删除账本字段不完整或包含未知字段。")
        if _integer(root.get("schemaVersion"), "schemaVersion") != 1:
            raise BackupError("账户删除账本版本不受支持。")
        raw_entries = root.get("entries")
        if not isinstance(raw_entries, list):
            raise BackupError("账户删除账本 entries 必须是数组。")
        entries = tuple(AccountDeletionLedgerEntry.from_mapping(item) for item in raw_entries)
        if entries != tuple(sorted(entries, key=lambda item: item.job_id)):
            raise BackupError("账户删除账本必须按 jobId 排序。")
        job_ids = {entry.job_id for entry in entries}
        principal_ids = {entry.principal_id for entry in entries}
        if len(job_ids) != len(entries) or len(principal_ids) != len(entries):
            raise BackupError("账户删除账本包含重复任务或主体。")
        return cls(entries)

    @classmethod
    def load(cls, path: Path) -> "AccountDeletionLedger":
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except FileNotFoundError as error:
            raise BackupError(f"账户删除账本不存在：{path.name}。") from error
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise BackupError("账户删除账本不是有效 UTF-8 JSON。") from error
        return cls.from_mapping(value)

    def to_mapping(self) -> dict[str, object]:
        return {
            "schemaVersion": 1,
            "entries": [entry.to_mapping() for entry in self.entries],
        }

    def merge(self, newer: "AccountDeletionLedger") -> "AccountDeletionLedger":
        by_job = {entry.job_id: entry for entry in self.entries}
        by_principal = {entry.principal_id: entry for entry in self.entries}
        for entry in newer.entries:
            existing_job = by_job.get(entry.job_id)
            existing_principal = by_principal.get(entry.principal_id)
            if existing_job is not None and existing_job != entry:
                raise BackupError("账户删除账本试图改写既有任务。")
            if existing_principal is not None and existing_principal != entry:
                raise BackupError("账户删除账本试图为同一主体写入第二个任务。")
            by_job[entry.job_id] = entry
            by_principal[entry.principal_id] = entry
        return AccountDeletionLedger(tuple(sorted(by_job.values(), key=lambda item: item.job_id)))


@dataclass(frozen=True, slots=True)
class BackupArtifact:
    path: str
    byte_length: int
    sha256: str


@dataclass(frozen=True, slots=True)
class BackupManifest:
    schema_version: int
    backup_id: str
    created_at: str
    server_name: str
    database_mode: str
    object_store_mode: str
    config_sha256: str
    rpo_minutes: int
    artifacts: tuple[BackupArtifact, ...]

    def to_mapping(self) -> dict[str, object]:
        return {
            "schemaVersion": self.schema_version,
            "backupId": self.backup_id,
            "createdAt": self.created_at,
            "serverName": self.server_name,
            "databaseMode": self.database_mode,
            "objectStoreMode": self.object_store_mode,
            "configSha256": self.config_sha256,
            "rpoMinutes": self.rpo_minutes,
            "artifacts": [
                {
                    "path": artifact.path,
                    "byteLength": artifact.byte_length,
                    "sha256": artifact.sha256,
                }
                for artifact in self.artifacts
            ],
        }

    @classmethod
    def from_mapping(cls, value: object) -> "BackupManifest":
        root = _mapping(value, "备份清单")
        expected = {
            "schemaVersion",
            "backupId",
            "createdAt",
            "serverName",
            "databaseMode",
            "objectStoreMode",
            "configSha256",
            "rpoMinutes",
            "artifacts",
        }
        if set(root) != expected:
            raise BackupError("备份清单字段不完整或包含未知字段。")
        schema_version = _integer(root.get("schemaVersion"), "schemaVersion")
        if schema_version != BACKUP_SCHEMA_VERSION:
            raise BackupError(f"不支持备份清单版本 {schema_version}。")
        backup_id = _text(root.get("backupId"), "backupId")
        if not BACKUP_ID.fullmatch(backup_id):
            raise BackupError("备份清单的 backupId 无效。")
        created_at = _text(root.get("createdAt"), "createdAt")
        _parse_utc(created_at)
        config_sha256 = _text(root.get("configSha256"), "configSha256")
        if not SHA256.fullmatch(config_sha256):
            raise BackupError("备份清单的配置摘要无效。")
        artifacts_value = root.get("artifacts")
        if not isinstance(artifacts_value, list) or not artifacts_value:
            raise BackupError("备份清单必须包含至少一个工件。")
        artifacts = tuple(_parse_artifact(item) for item in artifacts_value)
        paths = [artifact.path for artifact in artifacts]
        if paths != sorted(paths) or len(paths) != len(set(paths)):
            raise BackupError("备份工件路径必须唯一并按字典序排列。")
        return cls(
            schema_version=schema_version,
            backup_id=backup_id,
            created_at=created_at,
            server_name=_text(root.get("serverName"), "serverName"),
            database_mode=_text(root.get("databaseMode"), "databaseMode"),
            object_store_mode=_text(root.get("objectStoreMode"), "objectStoreMode"),
            config_sha256=config_sha256,
            rpo_minutes=_integer(root.get("rpoMinutes"), "rpoMinutes"),
            artifacts=artifacts,
        )


@dataclass(frozen=True, slots=True)
class BackupRepository:
    root: Path

    def prepare(self) -> None:
        resolved = self.root.expanduser().resolve()
        if not resolved.is_absolute() or resolved == Path(resolved.anchor):
            raise BackupError("备份仓库必须是非根绝对路径。")
        resolved.mkdir(mode=0o700, parents=True, exist_ok=True)
        _restrict_directory(resolved)
        (resolved / "wal").mkdir(mode=0o750, exist_ok=True)
        metrics = resolved / "metrics"
        metrics.mkdir(mode=0o755, exist_ok=True)
        metrics.chmod(0o755)

    def create_staging(self, backup_id: str) -> Path:
        _validate_backup_id(backup_id)
        self.prepare()
        staging = self.root / f".partial-{backup_id}"
        try:
            staging.mkdir(mode=0o700)
        except FileExistsError as error:
            raise BackupError(f"临时备份目录已存在：{backup_id}。") from error
        return staging

    def publish(self, manifest: BackupManifest) -> Path:
        _validate_backup_id(manifest.backup_id)
        staging = self.root / f".partial-{manifest.backup_id}"
        destination = self.root / manifest.backup_id
        if not staging.is_dir():
            raise BackupError("临时备份目录不存在。")
        if destination.exists():
            raise BackupError("同名正式备份已经存在。")
        _write_json(staging / MANIFEST_NAME, manifest.to_mapping())
        os.replace(staging, destination)
        latest = self.root / "LATEST"
        temporary = self.root / ".LATEST.tmp"
        temporary.write_text(manifest.backup_id + "\n", encoding="utf-8", newline="\n")
        os.replace(temporary, latest)
        return destination

    @property
    def account_deletion_ledger_path(self) -> Path:
        return self.root / ACCOUNT_DELETION_LEDGER_NAME

    def load_account_deletion_ledger(self) -> AccountDeletionLedger:
        path = self.account_deletion_ledger_path
        if not path.exists():
            return AccountDeletionLedger.empty()
        if path.is_symlink() or not path.is_file():
            raise BackupError("账户删除账本不是安全的常规文件。")
        return AccountDeletionLedger.load(path)

    def merge_account_deletion_ledger(self, snapshot: AccountDeletionLedger) -> AccountDeletionLedger:
        merged = self.load_account_deletion_ledger().merge(snapshot)
        _write_json(self.account_deletion_ledger_path, merged.to_mapping())
        return merged

    def write_metric_snapshot(self, name: str, lines: tuple[str, ...]) -> None:
        if name not in {"backup", "restore"}:
            raise BackupError("运行指标快照名称无效。")
        self.prepare()
        metrics = self.root / "metrics"
        destination = metrics / f"{name}.prom"
        temporary = metrics / f".{name}.prom.tmp"
        temporary.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        temporary.chmod(0o644)
        os.replace(temporary, destination)
        destination.chmod(0o644)

    def load(self, backup_id: str) -> BackupManifest:
        _validate_backup_id(backup_id)
        path = self.root / backup_id / MANIFEST_NAME
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except FileNotFoundError as error:
            raise BackupError(f"备份不存在：{backup_id}。") from error
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise BackupError("备份清单不是有效 UTF-8 JSON。") from error
        manifest = BackupManifest.from_mapping(value)
        if manifest.backup_id != backup_id:
            raise BackupError("目录名与备份清单 ID 不一致。")
        return manifest

    def verify(self, backup_id: str) -> BackupManifest:
        manifest = self.load(backup_id)
        directory = (self.root / backup_id).resolve()
        actual_paths = {
            path.relative_to(directory).as_posix()
            for path in _regular_files(directory)
            if path.name != MANIFEST_NAME
        }
        expected_paths = {artifact.path for artifact in manifest.artifacts}
        if actual_paths != expected_paths:
            missing = sorted(expected_paths - actual_paths)
            unexpected = sorted(actual_paths - expected_paths)
            raise BackupError(f"备份工件集合不一致；缺失={missing}，多余={unexpected}。")
        for artifact in manifest.artifacts:
            path = _safe_child(directory, artifact.path)
            stat = path.stat()
            if stat.st_size != artifact.byte_length or _sha256_file(path) != artifact.sha256:
                raise BackupError(f"备份工件摘要不匹配：{artifact.path}。")
        _require_restore_contract(manifest)
        return manifest

    def prune(
        self,
        retention_days: int,
        recent_retention_hours: int = 24,
        *,
        now: datetime | None = None,
    ) -> tuple[str, ...]:
        if not 7 <= retention_days <= 365:
            raise BackupError("备份保留天数必须在 7–365 之间。")
        if not 1 <= recent_retention_hours <= 168:
            raise BackupError("近期高频备份保留小时数必须在 1–168 之间。")
        reference = now or datetime.now(UTC)
        manifests: list[BackupManifest] = []
        for path in self.root.iterdir():
            if path.is_dir() and BACKUP_ID.fullmatch(path.name):
                manifests.append(self.load(path.name))
        manifests.sort(key=lambda item: _parse_utc(item.created_at), reverse=True)
        removed: list[str] = []
        cutoff = reference - timedelta(days=retention_days)
        recent_cutoff = reference - timedelta(hours=recent_retention_hours)
        retained_days = {
            _parse_utc(manifest.created_at).date()
            for manifest in manifests
            if _parse_utc(manifest.created_at) >= recent_cutoff
        }
        for manifest in manifests[1:]:
            created_at = _parse_utc(manifest.created_at)
            if created_at >= recent_cutoff:
                continue
            if created_at >= cutoff and created_at.date() not in retained_days:
                retained_days.add(created_at.date())
                continue
            try:
                shutil.rmtree(self.root / manifest.backup_id)
            except FileNotFoundError:
                # 计划任务与人工运维可能同时触发清理；另一进程已经删除目标时，
                # 当前清理已经达到期望终态，不应让完整备份流程失败。
                continue
            removed.append(manifest.backup_id)
        self._remove_stale_partials(reference)
        return tuple(removed)

    def require_headroom(self) -> int:
        self.prepare()
        latest_size = 0
        latest_path = self.root / "LATEST"
        if latest_path.is_symlink():
            raise BackupError("LATEST 不得是符号链接。")
        if latest_path.is_file():
            latest_id = latest_path.read_text(encoding="utf-8").strip()
            latest_size = sum(artifact.byte_length for artifact in self.load(latest_id).artifacts)
        required = max(MINIMUM_BACKUP_HEADROOM_BYTES, latest_size * 2)
        available = shutil.disk_usage(self.root).free
        if available < required:
            raise BackupError(
                f"备份前磁盘余量不足：可用 {available} 字节，至少需要 {required} 字节。"
            )
        return required

    def prune_archived_wal(self, manifest: BackupManifest) -> tuple[str, ...]:
        if manifest.database_mode != "embedded":
            return ()
        try:
            restore_point = RestorePoint.load(
                self.root / manifest.backup_id / "postgres" / "restore-point.json"
            )
        except RestorePointError as error:
            raise BackupError(str(error)) from error
        last_required_wal = restore_point.last_required_wal
        wal_directory = (self.root / "wal").resolve()
        try:
            wal_directory.relative_to(self.root.resolve())
        except ValueError as error:
            raise BackupError("归档 WAL 目录越过备份仓库。") from error
        if not wal_directory.is_dir():
            return ()

        removed: list[str] = []
        for path in sorted(wal_directory.iterdir(), key=lambda item: item.name):
            if path.is_symlink() or not path.is_file():
                raise BackupError(f"归档 WAL 中包含不安全条目：{path.name}。")
            marker = WAL_BACKUP_MARKER.fullmatch(path.name)
            if WAL_SEGMENT.fullmatch(path.name):
                segment = path.name
            else:
                segment = marker.group(1) if marker else None
            if segment is None or segment > last_required_wal:
                continue
            path.unlink()
            removed.append(path.name)
        return tuple(removed)

    def _remove_stale_partials(self, now: datetime) -> None:
        cutoff = now - LOCK_STALE_AFTER
        for path in self.root.glob(".partial-*"):
            if not path.is_dir():
                continue
            modified = datetime.fromtimestamp(path.stat().st_mtime, UTC)
            if modified < cutoff:
                shutil.rmtree(path)


@dataclass(slots=True)
class BackupCoordinator:
    config: DeploymentConfig
    paths: DeploymentPaths
    capture: BackupCapture
    repository: BackupRepository
    clock: Callable[[], datetime] = lambda: datetime.now(UTC)

    def create(self) -> BackupManifest:
        created_at = self.clock().astimezone(UTC)
        backup_id = _new_backup_id(created_at)
        with _repository_lock(self.repository.root, created_at):
            staging = self.repository.create_staging(backup_id)
            try:
                self._copy_identity_artifacts(staging)
                self.capture.capture_backup_payload(backup_id)
                deletion_snapshot = AccountDeletionLedger.load(staging / ACCOUNT_DELETION_ARTIFACT)
                self.repository.merge_account_deletion_ledger(deletion_snapshot)
                self._copy_provider_evidence(staging, created_at)
                artifacts = _inventory(staging)
                manifest = BackupManifest(
                    schema_version=BACKUP_SCHEMA_VERSION,
                    backup_id=backup_id,
                    created_at=created_at.isoformat().replace("+00:00", "Z"),
                    server_name=self.config.public.server_name,
                    database_mode=self.config.database.mode,
                    object_store_mode=self.config.object_store.mode,
                    config_sha256=_config_digest(self.config),
                    rpo_minutes=self.config.backup.rpo_minutes,
                    artifacts=artifacts,
                )
                _require_restore_contract(manifest)
                self.repository.publish(manifest)
                self.repository.verify(backup_id)
                self.repository.write_metric_snapshot(
                    "backup",
                    (
                        f"agent_room_backup_last_success_timestamp_seconds {created_at.timestamp():.3f}",
                        f"agent_room_backup_rpo_target_seconds {manifest.rpo_minutes * 60}",
                    ),
                )
                return manifest
            except BaseException:
                shutil.rmtree(staging, ignore_errors=True)
                raise

    def _copy_identity_artifacts(self, staging: Path) -> None:
        identity = staging / "identity"
        identity.mkdir(mode=0o700)
        signing_key = self.paths.data / "synapse" / f"{self.config.public.server_name}.signing.key"
        realm = self.paths.generated / "keycloak" / "realm-agent-room.json"
        for source, name in ((signing_key, "synapse.signing.key"), (realm, "keycloak-realm.json")):
            if not source.is_file() or source.is_symlink():
                raise BackupError(f"身份备份源缺失或不安全：{source.name}。")
            shutil.copyfile(source, identity / name)

    def _copy_provider_evidence(self, staging: Path, created_at: datetime) -> None:
        evidence_path = self.config.backup.provider_pitr_evidence_file
        if evidence_path is None:
            return
        source = Path(evidence_path)
        evidence = _load_provider_evidence(source, created_at, self.config.backup.rpo_minutes)
        _write_json(staging / "database" / "provider-pitr-evidence.json", evidence)


def _inventory(directory: Path) -> tuple[BackupArtifact, ...]:
    artifacts: list[BackupArtifact] = []
    for path in _regular_files(directory):
        relative = path.relative_to(directory).as_posix()
        stat = path.stat()
        artifacts.append(BackupArtifact(relative, stat.st_size, _sha256_file(path)))
    artifacts.sort(key=lambda item: item.path)
    if not artifacts:
        raise BackupError("备份没有产生任何工件。")
    return tuple(artifacts)


def _regular_files(directory: Path) -> Iterator[Path]:
    resolved = directory.resolve()
    for root, directories, files in os.walk(resolved, followlinks=False):
        root_path = Path(root)
        for name in tuple(directories):
            child = root_path / name
            if child.is_symlink():
                raise BackupError(f"备份中禁止目录符号链接：{child.name}。")
        for name in files:
            child = root_path / name
            if child.is_symlink() or not child.is_file():
                raise BackupError(f"备份中只允许常规文件：{child.name}。")
            child.resolve().relative_to(resolved)
            yield child


def _require_restore_contract(manifest: BackupManifest) -> None:
    paths = {artifact.path for artifact in manifest.artifacts}
    required = {
        "database/agent-room.dump",
        "database/synapse.dump",
        "database/keycloak.dump",
        "identity/synapse.signing.key",
        "identity/keycloak-realm.json",
        "objects/source-inventory.ndjson",
        ACCOUNT_DELETION_ARTIFACT,
    }
    if manifest.database_mode == "embedded":
        required.update({"postgres/base/backup_manifest", "postgres/restore-point.json"})
        if not any(path.startswith("postgres/wal/") for path in paths):
            raise BackupError("内置数据库备份缺少归档 WAL。")
    else:
        required.add("database/provider-pitr-evidence.json")
    missing = sorted(required - paths)
    if missing:
        raise BackupError(f"备份恢复契约不完整：{missing}。")


def _uuid_text(value: object, name: str, *, version: int | None = None) -> str:
    text = _text(value, name)
    try:
        parsed = UUID(text)
    except ValueError as error:
        raise BackupError(f"{name} 不是有效 UUID。") from error
    if str(parsed) != text.lower() or (version is not None and parsed.version != version):
        raise BackupError(f"{name} 不是规范 UUID" + (f"v{version}" if version else "") + "。")
    return str(parsed)


def _load_provider_evidence(path: Path, now: datetime, required_rpo: int) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise BackupError(f"供应商 PITR 证据不存在：{path}。") from error
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BackupError("供应商 PITR 证据不是有效 UTF-8 JSON。") from error
    root = _mapping(value, "供应商 PITR 证据")
    expected = {
        "schemaVersion",
        "provider",
        "cluster",
        "observedAt",
        "continuousRecoveryEnabled",
        "rpoMinutes",
    }
    if set(root) != expected:
        raise BackupError("供应商 PITR 证据字段不完整或包含未知字段。")
    if _integer(root.get("schemaVersion"), "schemaVersion") != 1:
        raise BackupError("供应商 PITR 证据版本不受支持。")
    if root.get("continuousRecoveryEnabled") is not True:
        raise BackupError("供应商 PITR 未启用。")
    reported_rpo = _integer(root.get("rpoMinutes"), "rpoMinutes")
    if reported_rpo > required_rpo:
        raise BackupError("供应商 PITR 的 RPO 未达到部署目标。")
    observed_at = _parse_utc(_text(root.get("observedAt"), "observedAt"))
    if observed_at > now + timedelta(minutes=1) or now - observed_at > timedelta(minutes=30):
        raise BackupError("供应商 PITR 证据不是最近 30 分钟内采集的。")
    _text(root.get("provider"), "provider")
    _text(root.get("cluster"), "cluster")
    return root


@contextmanager
def _repository_lock(root: Path, now: datetime) -> Iterator[None]:
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    lock = root / ".backup.lock"
    if lock.exists():
        modified = datetime.fromtimestamp(lock.stat().st_mtime, UTC)
        if now - modified <= LOCK_STALE_AFTER:
            raise BackupError("已有备份任务持有仓库锁。")
        lock.unlink()
    descriptor = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as stream:
            stream.write(json.dumps({"pid": os.getpid(), "createdAt": now.isoformat()}))
            stream.write("\n")
        yield
    finally:
        lock.unlink(missing_ok=True)


def _new_backup_id(now: datetime) -> str:
    return now.strftime("%Y%m%dT%H%M%S%fZ-") + secrets.token_hex(4)


def _config_digest(config: DeploymentConfig) -> str:
    value = asdict(config)
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _safe_child(root: Path, relative: str) -> Path:
    logical = PurePosixPath(relative)
    if logical.is_absolute() or ".." in logical.parts or "\\" in relative or not logical.parts:
        raise BackupError("备份工件路径不是安全的相对 POSIX 路径。")
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError as error:
        raise BackupError("备份工件路径越过备份目录。") from error
    return path


def _parse_artifact(value: object) -> BackupArtifact:
    source = _mapping(value, "备份工件")
    if set(source) != {"path", "byteLength", "sha256"}:
        raise BackupError("备份工件字段不完整或包含未知字段。")
    path = _text(source.get("path"), "path")
    _safe_child(Path("/").resolve(), path)
    byte_length = _integer(source.get("byteLength"), "byteLength")
    if byte_length < 0:
        raise BackupError("备份工件长度不能为负数。")
    digest = _text(source.get("sha256"), "sha256")
    if not SHA256.fullmatch(digest):
        raise BackupError("备份工件 SHA-256 无效。")
    return BackupArtifact(path, byte_length, digest)


def _parse_utc(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise BackupError("时间戳不是有效 ISO 8601。") from error
    if parsed.tzinfo is None or parsed.utcoffset() != timedelta(0):
        raise BackupError("时间戳必须使用 UTC。")
    return parsed.astimezone(UTC)


def _mapping(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise BackupError(f"{label} 必须是 JSON 对象。")
    return value


def _text(value: object, label: str) -> str:
    if not isinstance(value, str) or not value or len(value) > 4_096:
        raise BackupError(f"{label} 必须是非空短文本。")
    return value


def _integer(value: object, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise BackupError(f"{label} 必须是整数。")
    return value


def _validate_backup_id(value: str) -> None:
    if not BACKUP_ID.fullmatch(value):
        raise BackupError("备份 ID 格式无效。")


def _restrict_directory(path: Path) -> None:
    try:
        path.chmod(0o700)
    except OSError as error:
        raise BackupError(f"无法收紧备份目录权限：{path}。") from error


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    temporary.chmod(0o600)
    os.replace(temporary, path)
