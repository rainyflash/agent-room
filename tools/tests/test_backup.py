from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime, timedelta
import json
from pathlib import Path
import tempfile
import unittest

from tools.prodops.backup import (
    BackupCoordinator,
    BackupError,
    BackupManifest,
    BackupRepository,
)
from tools.prodops.config import BackupConfig, load_deployment_config
from tools.prodops.render import DeploymentPaths, render_deployment
from tools.prodops.secrets import SecretStore


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"
FIXED_NOW = datetime(2026, 8, 25, 12, 34, 56, 123456, tzinfo=UTC)


class FakeBackupCapture:
    def __init__(self, repository: Path, *, embedded: bool = True) -> None:
        self.repository = repository
        self.embedded = embedded

    def capture_backup_payload(self, backup_id: str) -> None:
        staging = self.repository / f".partial-{backup_id}"
        for name in ("agent-room.dump", "synapse.dump", "keycloak.dump"):
            write(staging / "database" / name, name.encode())
        write(staging / "objects" / "source-inventory.ndjson", b'{"key":"content/1"}\n')
        write(staging / "objects" / "data" / "content" / "1", b"payload")
        if self.embedded:
            write(staging / "postgres" / "base" / "backup_manifest", b"{}")
            write(staging / "postgres" / "restore-point.json", b"{}")
            write(staging / "postgres" / "wal" / "000000010000000000000001", b"wal")


class BackupCoordinatorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.state = root / "state"
        self.repository_path = root / "backups"
        self.paths = DeploymentPaths.from_state(self.state)
        base = load_deployment_config(EXAMPLE)
        self.config = replace(
            base,
            backup=replace(base.backup, repository=self.repository_path.as_posix()),
        )
        secrets = SecretStore(self.paths.secrets)
        secrets.initialize()
        render_deployment(self.config, self.paths, secrets)
        signing = self.paths.data / "synapse" / f"{self.config.public.server_name}.signing.key"
        write(signing, b"ed25519 signing identity")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_backup_is_atomically_published_and_verified(self) -> None:
        repository = BackupRepository(self.repository_path)
        coordinator = BackupCoordinator(
            self.config,
            self.paths,
            FakeBackupCapture(self.repository_path),
            repository,
            clock=lambda: FIXED_NOW,
        )

        manifest = coordinator.create()

        self.assertEqual(manifest.rpo_minutes, 15)
        self.assertEqual(repository.verify(manifest.backup_id), manifest)
        self.assertEqual(
            (self.repository_path / "LATEST").read_text(encoding="utf-8").strip(),
            manifest.backup_id,
        )
        self.assertFalse((self.repository_path / f".partial-{manifest.backup_id}").exists())

    def test_tampered_artifact_is_rejected(self) -> None:
        repository = BackupRepository(self.repository_path)
        manifest = BackupCoordinator(
            self.config,
            self.paths,
            FakeBackupCapture(self.repository_path),
            repository,
            clock=lambda: FIXED_NOW,
        ).create()
        write(self.repository_path / manifest.backup_id / "database" / "agent-room.dump", b"tampered")

        with self.assertRaisesRegex(BackupError, "摘要不匹配"):
            repository.verify(manifest.backup_id)

    def test_manifest_rejects_path_traversal(self) -> None:
        value = {
            "schemaVersion": 1,
            "backupId": "20260825T123456123456Z-0123abcd",
            "createdAt": "2026-08-25T12:34:56Z",
            "serverName": "agent-room.example",
            "databaseMode": "embedded",
            "objectStoreMode": "embedded",
            "configSha256": "0" * 64,
            "rpoMinutes": 15,
            "artifacts": [{"path": "../escape", "byteLength": 1, "sha256": "0" * 64}],
        }

        with self.assertRaisesRegex(BackupError, "安全的相对"):
            BackupManifest.from_mapping(value)

    def test_prune_keeps_newest_even_when_all_are_expired(self) -> None:
        repository = BackupRepository(self.repository_path)
        old = FIXED_NOW - timedelta(days=40)
        first = BackupCoordinator(
            self.config,
            self.paths,
            FakeBackupCapture(self.repository_path),
            repository,
            clock=lambda: old,
        ).create()
        second = BackupCoordinator(
            self.config,
            self.paths,
            FakeBackupCapture(self.repository_path),
            repository,
            clock=lambda: old + timedelta(hours=1),
        ).create()

        removed = repository.prune(30, now=FIXED_NOW)

        self.assertEqual(removed, (first.backup_id,))
        self.assertTrue((self.repository_path / second.backup_id).is_dir())

    def test_external_backup_requires_fresh_provider_evidence(self) -> None:
        evidence_path = Path(self.temporary.name) / "provider.json"
        evidence_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "provider": "测试云",
                    "cluster": "cluster-1",
                    "observedAt": (FIXED_NOW - timedelta(hours=1)).isoformat(),
                    "continuousRecoveryEnabled": True,
                    "rpoMinutes": 5,
                }
            ),
            encoding="utf-8",
        )
        external = replace(
            self.config,
            database=replace(self.config.database, mode="external"),
            backup=BackupConfig(
                repository=self.repository_path.as_posix(),
                retention_days=30,
                rpo_minutes=15,
                provider_pitr_evidence_file=evidence_path.as_posix(),
            ),
        )

        with self.assertRaisesRegex(BackupError, "最近 30 分钟"):
            BackupCoordinator(
                external,
                self.paths,
                FakeBackupCapture(self.repository_path, embedded=False),
                BackupRepository(self.repository_path),
                clock=lambda: FIXED_NOW,
            ).create()


def write(path: Path, content: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_bytes(content)


if __name__ == "__main__":
    unittest.main()
