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

from .config import DeploymentConfig
from .render import DeploymentPaths


BACKUP_SCHEMA_VERSION: Final = 1
BACKUP_ID: Final = re.compile(r"^[0-9]{8}T[0-9]{12}Z-[0-9a-f]{8}$")
SHA256: Final = re.compile(r"^[0-9a-f]{64}$")
LOCK_STALE_AFTER: Final = timedelta(hours=24)
MANIFEST_NAME: Final = "manifest.json"


class BackupError(RuntimeError):
    """表示备份不完整、被篡改或无法安全发布。"""


class BackupCapture(Protocol):
    def capture_backup_payload(self, backup_id: str) -> None:
        """把外部依赖快照写入指定的临时备份目录。"""


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

    def prune(self, retention_days: int, *, now: datetime | None = None) -> tuple[str, ...]:
        if not 7 <= retention_days <= 365:
            raise BackupError("备份保留天数必须在 7–365 之间。")
        reference = now or datetime.now(UTC)
        manifests: list[BackupManifest] = []
        for path in self.root.iterdir():
            if path.is_dir() and BACKUP_ID.fullmatch(path.name):
                manifests.append(self.load(path.name))
        manifests.sort(key=lambda item: _parse_utc(item.created_at), reverse=True)
        removed: list[str] = []
        cutoff = reference - timedelta(days=retention_days)
        for manifest in manifests[1:]:
            if _parse_utc(manifest.created_at) >= cutoff:
                continue
            shutil.rmtree(self.root / manifest.backup_id)
            removed.append(manifest.backup_id)
        self._remove_stale_partials(reference)
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
