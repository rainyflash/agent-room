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
from tools.prodops.restore_point import restore_point_name
from tools.prodops.secrets import SecretStore
from tools.prodops.wal_store import WalAnchor, WalPoint, WalStore, WalStoreError


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"
FIXED_NOW = datetime(2026, 8, 25, 12, 34, 56, 123456, tzinfo=UTC)


class FakeBackupCapture:
    """照 postgres-base-backup.sh 和 postgres-wal-archive.sh 写出的文件造一份。"""

    def __init__(self, repository: Path, *, embedded: bool = True, start_segment: int = 1) -> None:
        self.repository = repository
        self.embedded = embedded
        self.start_segment = start_segment
        self.archived: list[tuple[str, WalAnchor]] = []

    def capture_backup_payload(self, backup_id: str) -> None:
        staging = self.repository / f".partial-{backup_id}"
        for name in ("agent-room.dump", "synapse.dump", "keycloak.dump"):
            write(staging / "database" / name, name.encode())
        write(staging / "objects" / "source-inventory.ndjson", b'{"key":"content/1"}\n')
        write(staging / "objects" / "data" / "content" / "1", b"payload")
        write(staging / "privacy" / "account-deletions.json", b'{"schemaVersion":1,"entries":[]}\n')
        if self.embedded:
            start, last = segment(self.start_segment), segment(self.start_segment + 1)
            write(staging / "postgres" / "base" / "backup_manifest", b"{}")
            write(
                staging / "postgres" / "restore-point.json",
                json.dumps(
                    {"name": restore_point_name(backup_id), "lsn": "0/1000000", "lastRequiredWal": last}
                ).encode(),
            )
            write(staging / "postgres" / "wal" / start, b"wal")
            write(staging / "postgres" / "wal" / last, b"wal")

    def archive_wal(self, point_id: str, anchor: WalAnchor) -> None:
        self.archived.append((point_id, anchor))
        store = WalStore(self.repository / "wal-store")
        points = list(store.points())
        created_at = datetime.strptime(point_id[:22], "%Y%m%dT%H%M%S%fZ").replace(tzinfo=UTC)
        if not points:
            points.append(
                WalPoint(created_at, "anchor", anchor.name, anchor.lsn, anchor.segment, anchor.backup_id)
            )
        # 恢复点打在这套全量之后，所在的段排在链的末尾和全量自带的段后面。
        stored = segment(max(segment_number(points[-1].segment), segment_number(anchor.segment)) + 1)
        write(store.root / f"{stored}.gz", b"wal")
        write(store.root / f"{stored}.gz.sha256", b"digest")
        points.append(WalPoint(created_at, "point", restore_point_name(point_id), "0/2000000", stored, None))
        store._write(tuple(points))


def segment(number: int) -> str:
    """16 MB 一段时的段名：时间线 1，每 4 GB 256 段。"""

    return f"00000001{number >> 8:08X}{number & 0xFF:08X}"


def segment_number(name: str) -> int:
    return int(name[8:16], 16) * 256 + int(name[16:24], 16)


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
        capture = FakeBackupCapture(self.repository_path)
        coordinator = BackupCoordinator(
            self.config,
            self.paths,
            capture,
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
        # 第一次从这套全量的恢复点接起 WAL。
        [(point_id, anchor)] = capture.archived
        self.assertNotEqual(point_id, manifest.backup_id)
        self.assertEqual(
            anchor,
            WalAnchor(manifest.backup_id, restore_point_name(manifest.backup_id), "0/1000000", segment(2)),
        )
        points = repository.wal_store.points()
        self.assertEqual([point.kind for point in points], ["anchor", "point"])
        self.assertEqual(points[-1].name, restore_point_name(point_id))
        metrics = (self.repository_path / "metrics" / "backup.prom").read_text(encoding="utf-8")
        self.assertIn(
            f"agent_room_backup_last_success_timestamp_seconds {FIXED_NOW.timestamp():.3f}", metrics
        )
        self.assertIn("agent_room_backup_rpo_target_seconds 900", metrics)

    def test_later_backups_continue_the_same_wal_chain(self) -> None:
        repository = BackupRepository(self.repository_path)
        capture = FakeBackupCapture(self.repository_path)
        later = FIXED_NOW + timedelta(minutes=15)
        for moment in (FIXED_NOW, later):
            BackupCoordinator(
                self.config, self.paths, capture, repository, clock=lambda moment=moment: moment
            ).create()

        points = repository.wal_store.points()

        self.assertEqual([point.kind for point in points], ["anchor", "point", "point"])
        self.assertEqual(points[-1].created_at, later)
        metrics = (self.repository_path / "metrics" / "backup.prom").read_text(encoding="utf-8")
        self.assertIn(f"agent_room_backup_last_success_timestamp_seconds {later.timestamp():.3f}", metrics)

    def test_broken_wal_keeps_the_full_but_does_not_report_success(self) -> None:
        class BrokenWal(FakeBackupCapture):
            def archive_wal(self, point_id: str, anchor: WalAnchor) -> None:
                raise RuntimeError("WAL 读不通")

        repository = BackupRepository(self.repository_path)

        with self.assertRaisesRegex(RuntimeError, "读不通"):
            BackupCoordinator(
                self.config,
                self.paths,
                BrokenWal(self.repository_path),
                repository,
                clock=lambda: FIXED_NOW,
            ).create()

        # 全量已经发布、能用；但不更新“最近成功”，RPO 告警会叫人来看。
        latest = (self.repository_path / "LATEST").read_text(encoding="utf-8").strip()
        repository.verify(latest)
        self.assertFalse((self.repository_path / "metrics" / "backup.prom").exists())
        self.assertFalse((self.repository_path / ".backup.lock").exists())

    def test_wal_step_that_records_no_restore_point_fails(self) -> None:
        class SilentWal(FakeBackupCapture):
            def archive_wal(self, point_id: str, anchor: WalAnchor) -> None:
                pass

        with self.assertRaisesRegex(BackupError, "没有记下"):
            BackupCoordinator(
                self.config,
                self.paths,
                SilentWal(self.repository_path),
                BackupRepository(self.repository_path),
                clock=lambda: FIXED_NOW,
            ).create()

    def test_compressed_physical_backup_is_accepted_with_its_streamed_wal(self) -> None:
        class CompressedCapture(FakeBackupCapture):
            def capture_backup_payload(self, backup_id: str) -> None:
                super().capture_backup_payload(backup_id)
                base = self.repository / f".partial-{backup_id}" / "postgres" / "base"
                write(base / "base.tar.gz", b"base")
                write(base / "pg_wal.tar.gz", b"wal")

        repository = BackupRepository(self.repository_path)
        manifest = BackupCoordinator(
            self.config,
            self.paths,
            CompressedCapture(self.repository_path),
            repository,
            clock=lambda: FIXED_NOW,
        ).create()

        self.assertIn(
            "postgres/base/pg_wal.tar.gz", {artifact.path for artifact in manifest.artifacts}
        )
        self.assertEqual(repository.verify(manifest.backup_id), manifest)

    def test_compressed_physical_backup_without_streamed_wal_is_rejected(self) -> None:
        class MissingWalCapture(FakeBackupCapture):
            def capture_backup_payload(self, backup_id: str) -> None:
                super().capture_backup_payload(backup_id)
                base = self.repository / f".partial-{backup_id}" / "postgres" / "base"
                write(base / "base.tar.gz", b"base")

        coordinator = BackupCoordinator(
            self.config,
            self.paths,
            MissingWalCapture(self.repository_path),
            BackupRepository(self.repository_path),
            clock=lambda: FIXED_NOW,
        )

        with self.assertRaisesRegex(BackupError, "pg_wal.tar.gz"):
            coordinator.create()

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

    def test_wal_store_keeps_wal_from_the_oldest_retained_full_on(self) -> None:
        repository = BackupRepository(self.repository_path)
        for start, moment in ((1, FIXED_NOW - timedelta(days=40)), (5, FIXED_NOW - timedelta(days=1)), (9, FIXED_NOW)):
            BackupCoordinator(
                self.config,
                self.paths,
                FakeBackupCapture(self.repository_path, start_segment=start),
                repository,
                clock=lambda moment=moment: moment,
            ).create()
        self.assertEqual(repository.wal_store.segments(), (segment(3), segment(7), segment(11)))

        repository.prune(30, now=FIXED_NOW)
        removed = repository.prune_wal_store()

        # 40 天前那套删了；留下的最老一套从第 5 段起，之前的段和恢复点记录跟着删。
        self.assertEqual(removed, (segment(3),))
        self.assertEqual(repository.wal_store.segments(), (segment(7), segment(11)))
        self.assertFalse((self.repository_path / "wal-store" / f"{segment(3)}.gz.sha256").exists())
        self.assertEqual(
            [(point.kind, point.segment) for point in repository.wal_store.points()],
            [("point", segment(7)), ("point", segment(11))],
        )

    def test_wal_store_never_drops_the_last_two_restore_points(self) -> None:
        store = WalStore(self.repository_path / "wal-store")
        store.root.mkdir(parents=True)
        points = tuple(
            WalPoint(FIXED_NOW + timedelta(minutes=index), "point", f"agent_room_{index}", "0/1", segment(index), None)
            for index in (3, 4, 5)
        )
        store._write(points)
        for index in (3, 4, 5):
            write(store.root / f"{segment(index)}.gz", b"wal")

        # 下一次要从最后一个恢复点所在的段接着读，就算全量都比它新也不能删。
        self.assertEqual(store.prune(segment(9)), (segment(3),))
        self.assertEqual(store.points(), points[1:])
        self.assertEqual(store.segments(), (segment(4), segment(5)))

    def test_restore_point_log_is_parsed_strictly(self) -> None:
        store = WalStore(self.repository_path / "wal-store")
        store.root.mkdir(parents=True)
        good = WalPoint(FIXED_NOW, "point", "agent_room_x", "0/1", segment(2), None).to_line()
        for text in (
            good + " extra\n",
            good.replace("point", "anchor") + "\n",
            good.replace("agent_room_x", "agent-room'x") + "\n",
            WalPoint(FIXED_NOW, "point", "agent_room_y", "0/2", segment(3), None).to_line() + "\n" + good + "\n",
        ):
            store.log.write_text(text, encoding="utf-8")
            with self.subTest(text=text), self.assertRaises(WalStoreError):
                store.points()

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

    def test_external_database_relies_on_provider_pitr_instead_of_wal(self) -> None:
        evidence_path = Path(self.temporary.name) / "provider.json"
        evidence_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "provider": "测试云",
                    "cluster": "cluster-1",
                    "observedAt": FIXED_NOW.isoformat(),
                    "continuousRecoveryEnabled": True,
                    "rpoMinutes": 5,
                }
            ),
            encoding="utf-8",
        )
        external = replace(
            self.config,
            database=replace(self.config.database, mode="external"),
            backup=replace(self.config.backup, provider_pitr_evidence_file=evidence_path.as_posix()),
        )
        capture = FakeBackupCapture(self.repository_path, embedded=False)

        BackupCoordinator(
            external, self.paths, capture, BackupRepository(self.repository_path), clock=lambda: FIXED_NOW
        ).create()

        self.assertEqual(capture.archived, [])
        metrics = (self.repository_path / "metrics" / "backup.prom").read_text(encoding="utf-8")
        self.assertIn(
            f"agent_room_backup_last_success_timestamp_seconds {FIXED_NOW.timestamp():.3f}", metrics
        )


def write(path: Path, content: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_bytes(content)


if __name__ == "__main__":
    unittest.main()
