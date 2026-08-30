from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime, timedelta
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

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
        write(staging / "privacy" / "account-deletions.json", b'{"schemaVersion":1,"entries":[]}\n')
        if self.embedded:
            write(staging / "postgres" / "base" / "backup_manifest", b"{}")
            write(
                staging / "postgres" / "restore-point.json",
                (
                    b'{"name":"agent_room_test","lsn":"0/1000000",'
                    b'"lastRequiredWal":"000000010000000000000001"}\n'
                ),
            )
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
        metrics = (self.repository_path / "metrics" / "backup.prom").read_text(encoding="utf-8")
        self.assertIn("agent_room_backup_last_success_timestamp_seconds", metrics)
        self.assertIn("agent_room_backup_rpo_target_seconds 900", metrics)

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

    def test_prune_tolerates_target_removed_by_concurrent_cleanup(self) -> None:
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
        original_rmtree = shutil.rmtree

        def remove_then_report_missing(path: Path) -> None:
            original_rmtree(path)
            raise FileNotFoundError(path)

        with patch("tools.prodops.backup.shutil.rmtree", side_effect=remove_then_report_missing):
            removed = repository.prune(30, now=FIXED_NOW)

        self.assertEqual(removed, ())
        self.assertFalse((self.repository_path / first.backup_id).exists())
        self.assertTrue((self.repository_path / second.backup_id).is_dir())

    def test_account_deletion_ledger_is_monotonic_and_survives_pruning(self) -> None:
        repository = BackupRepository(self.repository_path)
        capture = FakeBackupCapture(self.repository_path)
        first = BackupCoordinator(
            self.config, self.paths, capture, repository, clock=lambda: FIXED_NOW - timedelta(days=40)
        ).create()
        entry = {
            "jobId": "019d2b8c-9100-7000-8000-000000000001",
            "principalId": "019d2b8c-9100-7000-8000-000000000002",
            "matrixUserId": "@deleted:agent-room.example",
            "completedAt": "2026-08-25T12:00:00Z",
        }

        class DeletionCapture(FakeBackupCapture):
            def capture_backup_payload(self, backup_id: str) -> None:
                super().capture_backup_payload(backup_id)
                write(
                    self.repository / f".partial-{backup_id}" / "privacy" / "account-deletions.json",
                    (json.dumps({"schemaVersion": 1, "entries": [entry]}) + "\n").encode(),
                )

        second = BackupCoordinator(
            self.config,
            self.paths,
            DeletionCapture(self.repository_path),
            repository,
            clock=lambda: FIXED_NOW,
        ).create()
        repository.prune(30, now=FIXED_NOW)

        self.assertFalse((self.repository_path / first.backup_id).exists())
        self.assertTrue((self.repository_path / second.backup_id).exists())
        self.assertEqual(repository.load_account_deletion_ledger().entries[0].job_id, entry["jobId"])

    def test_prune_keeps_recent_snapshots_and_one_daily_snapshot(self) -> None:
        repository = BackupRepository(self.repository_path)
        capture = FakeBackupCapture(self.repository_path)
        expired = BackupCoordinator(
            self.config,
            self.paths,
            capture,
            repository,
            clock=lambda: FIXED_NOW - timedelta(days=8),
        ).create()
        older_daily = BackupCoordinator(
            self.config,
            self.paths,
            capture,
            repository,
            clock=lambda: FIXED_NOW - timedelta(hours=26),
        ).create()
        retained_daily = BackupCoordinator(
            self.config,
            self.paths,
            capture,
            repository,
            clock=lambda: FIXED_NOW - timedelta(hours=25),
        ).create()
        recent = BackupCoordinator(
            self.config,
            self.paths,
            capture,
            repository,
            clock=lambda: FIXED_NOW - timedelta(hours=2),
        ).create()

        removed = repository.prune(7, 24, now=FIXED_NOW)

        self.assertEqual(set(removed), {expired.backup_id, older_daily.backup_id})
        self.assertTrue((self.repository_path / retained_daily.backup_id).is_dir())
        self.assertTrue((self.repository_path / recent.backup_id).is_dir())

    def test_archived_wal_is_pruned_only_through_published_restore_point(self) -> None:
        repository = BackupRepository(self.repository_path)
        manifest = BackupCoordinator(
            self.config,
            self.paths,
            FakeBackupCapture(self.repository_path),
            repository,
            clock=lambda: FIXED_NOW,
        ).create()
        wal_directory = self.repository_path / "wal"
        for name in (
            "000000010000000000000000",
            "000000010000000000000001",
            "000000010000000000000001.00000028.backup",
            "000000010000000000000002",
        ):
            write(wal_directory / name, name.encode())

        removed = repository.prune_archived_wal(manifest)

        self.assertEqual(
            set(removed),
            {
                "000000010000000000000000",
                "000000010000000000000001",
                "000000010000000000000001.00000028.backup",
            },
        )
        self.assertTrue((wal_directory / "000000010000000000000002").is_file())

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
                recent_retention_hours=24,
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
