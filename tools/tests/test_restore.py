from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime, timedelta
from pathlib import Path
import tempfile
import unittest

from tools.prodops.backup import BackupCoordinator, BackupRepository
from tools.prodops.config import load_deployment_config
from tools.prodops.render import DeploymentPaths, render_deployment
from tools.prodops.restore import (
    RETAINED_RESTORE_DRILLS,
    DatabaseRestoreEvidence,
    RestoreDrillCoordinator,
    RestoreDrillError,
    _prune_restore_drills,
)
from tools.prodops.secrets import SecretStore


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"
START = datetime(2026, 8, 25, 13, 0, 0, tzinfo=UTC)


class RestoreFixtureCapture:
    def __init__(self, repository: Path) -> None:
        self.repository = repository

    def capture_backup_payload(self, backup_id: str) -> None:
        staging = self.repository / f".partial-{backup_id}"
        for name in ("agent-room.dump", "synapse.dump", "keycloak.dump"):
            write(staging / "database" / name, name.encode())
        write(staging / "objects" / "source-inventory.ndjson", b"{}\n")
        write(staging / "objects" / "data" / "content.bin", b"object")
        write(staging / "privacy" / "account-deletions.json", b'{"schemaVersion":1,"entries":[]}\n')
        write(staging / "postgres" / "base" / "backup_manifest", b"{}")
        write(
            staging / "postgres" / "restore-point.json",
            b'{"name":"agent_room_point","lsn":"0/16B6C50",'
            b'"lastRequiredWal":"000000010000000000000001"}',
        )
        write(staging / "postgres" / "wal" / "000000010000000000000001", b"wal")


class FakeRestoreBackend:
    def restore_database(
        self,
        backup_directory: Path,
        drill_directory: Path,
        restore_point_name: str,
        restore_point_lsn: str,
        account_deletion_ledger: Path,
    ) -> DatabaseRestoreEvidence:
        self.backup_directory = backup_directory
        self.drill_directory = drill_directory
        self.account_deletion_ledger = account_deletion_ledger
        return DatabaseRestoreEvidence(
            restore_point_name,
            restore_point_lsn,
            True,
            3,
            ("agent_room", "keycloak", "synapse"),
            2,
            1,
            0,
            0,
        )


class RestoreDrillTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.paths = DeploymentPaths.from_state(root / "state")
        self.repository_path = root / "backups"
        base = load_deployment_config(EXAMPLE)
        self.config = replace(
            base,
            backup=replace(base.backup, repository=self.repository_path.as_posix()),
        )
        secret_store = SecretStore(self.paths.secrets)
        secret_store.initialize()
        render_deployment(self.config, self.paths, secret_store)
        signing = self.paths.data / "synapse" / f"{self.config.public.server_name}.signing.key"
        write(signing, b"stable signing identity")
        self.repository = BackupRepository(self.repository_path)
        self.manifest = BackupCoordinator(
            self.config,
            self.paths,
            RestoreFixtureCapture(self.repository_path),
            self.repository,
            clock=lambda: START,
        ).create()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_isolated_drill_restores_identity_objects_and_database(self) -> None:
        times = iter((START, START + timedelta(seconds=12)))
        backend = FakeRestoreBackend()

        report = RestoreDrillCoordinator(
            self.config,
            self.paths,
            self.repository,
            backend,
            clock=lambda: next(times),
        ).run(self.manifest.backup_id)

        self.assertTrue(report.rto_met)
        self.assertEqual(report.duration_seconds, 12)
        self.assertEqual(report.object_count, 1)
        self.assertEqual(report.object_bytes, 6)
        self.assertEqual(report.database.projection_memberships, 2)
        self.assertTrue((backend.drill_directory / "report.json").is_file())
        self.assertTrue((backend.drill_directory / "identity" / "synapse.signing.key").is_file())
        self.assertTrue(backend.account_deletion_ledger.is_file())
        metrics = (self.repository_path / "metrics" / "restore.prom").read_text(encoding="utf-8")
        self.assertIn("agent_room_restore_drill_duration_seconds 12.000", metrics)

    def test_external_database_cannot_claim_local_pitr_drill(self) -> None:
        external = replace(self.config, database=replace(self.config.database, mode="external"))

        with self.assertRaisesRegex(RestoreDrillError, "供应商隔离恢复"):
            RestoreDrillCoordinator(
                external,
                self.paths,
                self.repository,
                FakeRestoreBackend(),
            ).run(self.manifest.backup_id)


class RestoreDrillRetentionTests(unittest.TestCase):
    """演练目录与一次备份同量级，必须自动收敛，否则生产磁盘余量门禁最终会拒绝安装。"""

    @staticmethod
    def _drills(root: Path, count: int) -> list[Path]:
        created = []
        for index in range(count):
            path = root / f"20260916T{index:02d}0000000000Z-abcdef12-20260916T{index:02d}0100Z"
            path.mkdir()
            (path / "payload").write_bytes(b"x")
            created.append(path)
        return created

    def test_保留最近若干次且当前演练不会被删除(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            drills = self._drills(root, RETAINED_RESTORE_DRILLS + 4)
            current = drills[-1]

            removed = _prune_restore_drills(root, keep=current)

            remaining = sorted(path.name for path in root.iterdir() if path.is_dir())
            self.assertEqual(len(remaining), RETAINED_RESTORE_DRILLS)
            self.assertIn(current.name, remaining)
            self.assertEqual(sorted(removed), sorted(path.name for path in drills[:4]))

    def test_未超出保留数量时不删除任何演练(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            drills = self._drills(root, RETAINED_RESTORE_DRILLS)

            self.assertEqual(_prune_restore_drills(root, keep=drills[-1]), ())
            self.assertEqual(len(list(root.iterdir())), RETAINED_RESTORE_DRILLS)

    def test_不触碰演练目录以外的文件(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            drills = self._drills(root, RETAINED_RESTORE_DRILLS + 1)
            unrelated = root / "notes.txt"
            unrelated.write_text("keep", encoding="utf-8")

            _prune_restore_drills(root, keep=drills[-1])

            self.assertTrue(unrelated.is_file())


def write(path: Path, content: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_bytes(content)


if __name__ == "__main__":
    unittest.main()
