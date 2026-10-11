from __future__ import annotations

from dataclasses import replace
from datetime import UTC, datetime, timedelta
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

from tools.prodops.backup import BackupCoordinator, BackupRepository
from tools.prodops.config import load_deployment_config
from tools.prodops.render import DeploymentPaths, render_deployment
from tools.prodops.restore import (
    RETAINED_RESTORE_DRILLS,
    UNREACHED_TARGET,
    BaseCandidate,
    DatabaseRestoreEvidence,
    RecoveryTarget,
    RestoreDrillCoordinator,
    RestoreDrillError,
    RestoreTarget,
    _prune_restore_drills,
    materialize_base_backup,
    parse_target_time,
    plan_restore,
    prune_expired_restore_drills,
    read_object_inventory,
    restore_drill_root,
)
from tools.prodops.restore_point import RestorePoint, restore_point_name
from tools.prodops.secrets import SecretStore
from tools.prodops.wal_store import WalAnchor, WalPoint, WalStore


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"
START = datetime(2026, 8, 25, 13, 0, 0, tzinfo=UTC)


class RestoreFixtureCapture:
    """照 object-backup.sh 写出的样子：对象同步进仓库的镜像，快照只带清单。"""

    def __init__(self, repository: Path, *, copies_objects: bool = False) -> None:
        self.repository = repository
        self.copies_objects = copies_objects

    def capture_backup_payload(self, backup_id: str) -> None:
        staging = self.repository / f".partial-{backup_id}"
        for name in ("agent-room.dump", "synapse.dump", "keycloak.dump"):
            write(staging / "database" / name, name.encode())
        if self.copies_objects:
            # 改成增量同步以前：每套快照把对象存储整份复制一遍。
            write(staging / "objects" / "source-inventory.ndjson", b"{}\n")
            write(staging / "objects" / "data" / "content.bin", b"object")
        else:
            write(self.repository / "objects" / "mirror" / "content.bin", b"object")
            write(staging / "objects" / "source-inventory.ndjson", inventory_line("content.bin", b"object"))
        write(staging / "privacy" / "account-deletions.json", b'{"schemaVersion":1,"entries":[]}\n')
        write(staging / "postgres" / "base" / "backup_manifest", b"{}")
        write(
            staging / "postgres" / "restore-point.json",
            b'{"name":"agent_room_point","lsn":"0/16B6C50",'
            b'"lastRequiredWal":"000000010000000000000001"}',
        )
        write(staging / "postgres" / "wal" / "000000010000000000000001", b"wal")

    def archive_wal(self, point_id: str, anchor: WalAnchor) -> None:
        store = WalStore(self.repository / "wal-store")
        store_segment(store, "000000010000000000000002")
        store._write(
            (
                WalPoint(START, "anchor", anchor.name, anchor.lsn, anchor.segment, anchor.backup_id),
                WalPoint(START, "point", restore_point_name(point_id), "0/2000000", "000000010000000000000002", None),
            )
        )


class FakeRestoreBackend:
    def restore_database(
        self,
        backup_directory: Path,
        drill_directory: Path,
        recovery: RecoveryTarget,
        account_deletion_ledger: Path,
    ) -> DatabaseRestoreEvidence:
        self.backup_directory = backup_directory
        self.drill_directory = drill_directory
        self.recovery = recovery
        self.wal = sorted(path.name for path in (drill_directory / "wal").iterdir())
        self.account_deletion_ledger = account_deletion_ledger
        return DatabaseRestoreEvidence(
            recovery.name,
            recovery.lsn,
            True,
            3,
            ("agent_room", "keycloak", "synapse"),
            2,
            1,
            0,
            0,
            recovery.time_text,
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
        ).run(RestoreTarget(backup_id=self.manifest.backup_id))

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
        # 按快照恢复只用它自带的 WAL，停在它自己的恢复点。
        self.assertEqual(backend.recovery, RecoveryTarget(name="agent_room_point", lsn="0/16B6C50"))
        self.assertEqual(backend.wal, ["000000010000000000000001"])

    def test_by_default_the_drill_replays_kept_wal_to_the_latest_restore_point(self) -> None:
        report = self.drill().run()

        point = WalStore(self.repository_path / "wal-store").points()[-1]
        self.assertEqual(report.backup_id, self.manifest.backup_id)
        self.assertEqual(self.backend.recovery, RecoveryTarget(name=point.name, lsn="0/2000000"))
        # 快照自带的段原样放着，接在后面的段从 wal-store/ 取压缩过的。
        self.assertEqual(self.backend.wal, ["000000010000000000000001", "000000010000000000000002.gz"])
        self.assertEqual(report.to_mapping()["database"]["restorePointName"], point.name)

    def test_drill_refuses_kept_wal_that_changed_after_it_was_stored(self) -> None:
        write(self.repository_path / "wal-store" / "000000010000000000000002.gz", b"tampered")

        with self.assertRaisesRegex(RestoreDrillError, "SHA-256 对不上"):
            self.drill().run()
        (drill,) = restore_drill_root(self.paths).iterdir()
        self.assertTrue((drill / "FAILED").is_file())

    def drill(self) -> RestoreDrillCoordinator:
        times = iter((START, START + timedelta(seconds=12)))
        self.backend = FakeRestoreBackend()
        return RestoreDrillCoordinator(
            self.config, self.paths, self.repository, self.backend, clock=lambda: next(times)
        )

    def test_objects_removed_or_replaced_after_the_snapshot_come_back_from_that_day(self) -> None:
        objects = self.repository_path / "objects"
        # 快照之后同步时，这个对象被覆盖，旧版本挪进了当天的目录。
        write(objects / "removed" / START.date().isoformat() / "content.bin", b"object")
        write(objects / "mirror" / "content.bin", b"replaced")
        # 快照那天以前挪走的不算：那时它还不是这个样子。
        write(objects / "removed" / "2026-08-24" / "content.bin", b"stale")

        report = self.drill().run(RestoreTarget(backup_id=self.manifest.backup_id))

        drill = restore_drill_root(self.paths) / f"{self.manifest.backup_id}-{START:%Y%m%dT%H%M%SZ}"
        self.assertEqual((report.object_count, report.object_bytes), (1, 6))
        self.assertEqual((drill / "objects" / "content.bin").read_bytes(), b"object")

    def test_drill_fails_when_an_object_cannot_be_found(self) -> None:
        objects = self.repository_path / "objects"
        (objects / "mirror" / "content.bin").unlink()
        write(objects / "removed" / "2026-08-24" / "content.bin", b"object")

        with self.assertRaisesRegex(RestoreDrillError, "取不回来：content.bin"):
            self.drill().run(RestoreTarget(backup_id=self.manifest.backup_id))

    def test_inventory_paths_must_stay_inside_the_mirror(self) -> None:
        inventory = self.repository_path / "inventory.ndjson"
        for path in ("../escape", "/etc/passwd", "content/../../escape", "content\\1", ""):
            inventory.write_bytes(inventory_line(path, b"object"))
            with self.subTest(path=path), self.assertRaisesRegex(RestoreDrillError, "字段不对"):
                read_object_inventory(inventory)

    def test_older_snapshots_restore_their_own_object_copies(self) -> None:
        manifest = BackupCoordinator(
            self.config,
            self.paths,
            RestoreFixtureCapture(self.repository_path, copies_objects=True),
            self.repository,
            clock=lambda: START - timedelta(hours=1),
        ).create()
        (self.repository_path / "objects" / "mirror" / "content.bin").unlink()

        report = self.drill().run(RestoreTarget(backup_id=manifest.backup_id))

        self.assertEqual((report.object_count, report.object_bytes), (1, 6))

    def test_external_database_cannot_claim_local_pitr_drill(self) -> None:
        external = replace(self.config, database=replace(self.config.database, mode="external"))

        with self.assertRaisesRegex(RestoreDrillError, "供应商隔离恢复"):
            RestoreDrillCoordinator(
                external,
                self.paths,
                self.repository,
                FakeRestoreBackend(),
            ).run(RestoreTarget(backup_id=self.manifest.backup_id))


def segment(number: int) -> str:
    return f"{1:08X}{0:08X}{number:08X}"


def candidate(backup_id: str, name: str, lsn: str, last_wal: int) -> BaseCandidate:
    return BaseCandidate(backup_id, RestorePoint(name, lsn, segment(last_wal)))


def at(minute: int, second: int = 10) -> datetime:
    return datetime(2026, 10, 11, 9, minute, second, tzinfo=UTC)


class RestorePlanTests(unittest.TestCase):
    """按恢复点名字、按时间恢复时找哪套快照当起点、接哪些段。"""

    OLD = candidate("20261011T084500000000Z-0000000a", "agent_room_old", "0/1000100", 1)
    FIRST = candidate("20261011T090000000000Z-0000000b", "agent_room_first", "0/3000100", 3)
    SECOND = candidate("20261011T093000000000Z-0000000c", "agent_room_second", "0/6000100", 6)

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.store = WalStore(Path(self.temporary.name) / "wal-store")
        self.store.root.mkdir()
        for number in range(3, 10):
            store_segment(self.store, segment(number))
        # 链从 FIRST 接起，之后每 15 分钟一个恢复点；SECOND 在 09:30 前一点做完。
        self.points = (
            WalPoint(at(0), "anchor", "agent_room_first", "0/3000100", segment(3), self.FIRST.backup_id),
            WalPoint(at(15), "point", "agent_room_p1", "0/5000100", segment(5), None),
            WalPoint(at(30), "point", "agent_room_p2", "0/7000100", segment(7), None),
            WalPoint(at(45), "point", "agent_room_p3", "0/9000100", segment(9), None),
        )
        self.store._write(self.points)
        self.candidates = (self.OLD, self.FIRST, self.SECOND)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def plan(self, target: RestoreTarget):
        return plan_restore(target, self.candidates, self.store)

    def test_named_point_starts_from_the_latest_snapshot_before_it(self) -> None:
        early = self.plan(RestoreTarget(restore_point="agent_room_p1"))
        late = self.plan(RestoreTarget(restore_point="agent_room_p3"))

        self.assertEqual(early.base, self.FIRST.backup_id)
        self.assertEqual(early.wal, (segment(4), segment(5)))
        self.assertEqual(early.recovery, RecoveryTarget(name="agent_room_p1", lsn="0/5000100"))
        self.assertEqual(late.base, self.SECOND.backup_id)
        self.assertEqual(late.wal, (segment(7), segment(8), segment(9)))

    def test_without_a_target_the_latest_point_is_used(self) -> None:
        self.assertEqual(self.plan(RestoreTarget()), self.plan(RestoreTarget(restore_point="agent_room_p3")))

    def test_a_snapshots_own_restore_point_needs_no_kept_wal(self) -> None:
        plan = self.plan(RestoreTarget(restore_point="agent_room_second"))

        self.assertEqual((plan.base, plan.wal), (self.SECOND.backup_id, ()))

    def test_time_starts_before_it_and_keeps_wal_through_the_next_point(self) -> None:
        between = self.plan(RestoreTarget(time=at(20, 0)))
        later = self.plan(RestoreTarget(time=at(40, 0)))

        # 09:20 时 SECOND 还没做，从 FIRST 起；WAL 放到 09:30 的恢复点为止。
        self.assertEqual(between.base, self.FIRST.backup_id)
        self.assertEqual(between.wal, tuple(segment(number) for number in range(4, 8)))
        self.assertEqual(between.recovery, RecoveryTarget(time=at(20, 0), next_point="agent_room_p2"))
        self.assertEqual(later.base, self.SECOND.backup_id)
        self.assertEqual(later.wal, (segment(7), segment(8), segment(9)))

    def test_time_outside_the_kept_wal_is_refused(self) -> None:
        with self.assertRaisesRegex(RestoreDrillError, "晚于最近一个核对过的恢复点"):
            self.plan(RestoreTarget(time=at(50)))
        with self.assertRaisesRegex(RestoreDrillError, "早于最早的恢复点"):
            self.plan(RestoreTarget(time=at(0, 0)))

    def test_snapshots_from_before_the_chain_are_never_a_starting_point(self) -> None:
        self.candidates = (self.OLD,)

        with self.assertRaisesRegex(RestoreDrillError, "没有接得上的快照"):
            self.plan(RestoreTarget(restore_point="agent_room_p1"))

    def test_once_the_anchor_is_pruned_every_snapshot_left_is_on_the_chain(self) -> None:
        self.store._write(self.points[1:])
        self.candidates = (self.FIRST, self.SECOND)

        self.assertEqual(self.plan(RestoreTarget(restore_point="agent_room_p1")).base, self.FIRST.backup_id)

    def test_unknown_restore_point_is_refused(self) -> None:
        with self.assertRaisesRegex(RestoreDrillError, "没有这个恢复点：agent_room_typo"):
            self.plan(RestoreTarget(restore_point="agent_room_typo"))

    def test_no_restore_points_yet(self) -> None:
        self.store.log.unlink()

        with self.assertRaisesRegex(RestoreDrillError, "还没有核对过的恢复点"):
            self.plan(RestoreTarget())


class RecoveryTargetTests(unittest.TestCase):
    def test_settings_for_a_named_point_and_for_a_moment(self) -> None:
        named = RecoveryTarget(name="agent_room_p1", lsn="0/5000100").settings()
        timed = RecoveryTarget(time=at(20, 0), next_point="agent_room_p2").settings()

        self.assertIn("recovery_target_name = 'agent_room_p1'", named)
        self.assertIn("recovery_target_time = '2026-10-11 09:20:00.000000+00'", timed)
        for settings in (named, timed):
            self.assertIn("recovery_target_action = 'promote'", settings)
            self.assertTrue(settings[0].startswith("restore_command = "))

    def test_reaching_the_target(self) -> None:
        named = RecoveryTarget(name="agent_room_p1", lsn="0/5000100")
        timed = RecoveryTarget(time=at(20, 0), next_point="agent_room_p2")
        stopped = "LOG:  recovery stopping before commit of transaction 755, time 2026-10-11 09:20:01+00"

        self.assertTrue(named.reached("", replayed_past_lsn=True))
        self.assertTrue(named.reached('LOG:  recovery stopping at restore point "agent_room_p1"', replayed_past_lsn=False))
        self.assertFalse(named.reached("", replayed_past_lsn=False))
        self.assertTrue(timed.reached(stopped, replayed_past_lsn=False))
        self.assertFalse(timed.reached("LOG:  redo done", replayed_past_lsn=True))

    def test_explains_a_target_the_wal_never_reaches(self) -> None:
        logs = f"FATAL:  {UNREACHED_TARGET}"
        timed = RecoveryTarget(time=at(20, 0), next_point="agent_room_p2")

        self.assertIn("--restore-point agent_room_p2", timed.unreachable(logs) or "")
        self.assertIn("接不上", RecoveryTarget(name="agent_room_p1", lsn="0/5000100").unreachable(logs) or "")
        self.assertIsNone(timed.unreachable("FATAL:  could not open file"))

    def test_target_options(self) -> None:
        self.assertEqual(parse_target_time("2026-10-11T17:20:00+08:00"), at(20, 0))
        for text in ("2026-10-11T09:20:00", "yesterday"):
            with self.subTest(text=text), self.assertRaises(RestoreDrillError):
                parse_target_time(text)
        with self.assertRaisesRegex(RestoreDrillError, "只能给一个"):
            RestoreTarget(backup_id="20261011T090000000000Z-0000000b", restore_point="agent_room_p1")
        with self.assertRaisesRegex(RestoreDrillError, "带时区"):
            RestoreTarget(time=datetime(2026, 10, 11, 9, 20))


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

    def test_所恢复的备份超过保留期的演练目录跟着删(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            expired = root / "20260901T120000000000Z-abcdef12-20260902T080000Z"
            recent = root / "20260920T120000000000Z-abcdef12-20260920T130000Z"
            unrelated = root / "manual-copy"
            for path in (expired, recent, unrelated):
                path.mkdir()
                (path / "payload").write_bytes(b"x")

            removed = prune_expired_restore_drills(
                root, 30, datetime(2026, 10, 9, 8, 0, tzinfo=UTC)
            )

            self.assertEqual(removed, (expired.name,))
            self.assertFalse(expired.exists())
            self.assertTrue(recent.is_dir())
            self.assertTrue(unrelated.is_dir())

    def test_还没有演练目录时什么都不做(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            missing = Path(temporary) / "restore-drills"

            self.assertEqual(prune_expired_restore_drills(missing, 30, START), ())


class BaseBackupMaterializationTests(unittest.TestCase):
    """物理备份 2026-10-09 起改成 gzip 压缩的 tar；保留期内以前的普通目录也要能恢复。"""

    def test_tar_格式解成数据目录并把流式_WAL_放进_pg_wal(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "base"
            write_tar_gz(
                source / "base.tar.gz",
                {
                    "backup_label": b"START WAL LOCATION: 0/9000028\n",
                    "PG_VERSION": b"18\n",
                    "global/pg_control": b"control",
                    "pg_wal/": None,
                },
            )
            write_tar_gz(source / "pg_wal.tar.gz", {"000000010000000000000009": b"wal"})
            write(source / "backup_manifest", b"{}")
            target = root / "restored"

            materialize_base_backup(source, target)

            self.assertEqual((target / "PG_VERSION").read_bytes(), b"18\n")
            self.assertEqual((target / "global" / "pg_control").read_bytes(), b"control")
            self.assertEqual(
                (target / "pg_wal" / "000000010000000000000009").read_bytes(), b"wal"
            )

    def test_以前的普通目录格式照旧整份复制(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "base"
            write(source / "PG_VERSION", b"18\n")
            write(source / "pg_wal" / "000000010000000000000009", b"wal")
            target = root / "restored"

            materialize_base_backup(source, target)

            self.assertEqual((target / "PG_VERSION").read_bytes(), b"18\n")
            self.assertTrue((target / "pg_wal" / "000000010000000000000009").is_file())

    def test_tar_格式缺流式_WAL_时拒绝(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "base"
            write_tar_gz(source / "base.tar.gz", {"PG_VERSION": b"18\n"})

            with self.assertRaisesRegex(RestoreDrillError, "pg_wal.tar.gz"):
                materialize_base_backup(source, root / "restored")

    def test_压缩包里越出数据目录的条目被拒绝(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "base"
            write_tar_gz(source / "base.tar.gz", {"../escape": b"x"})
            write_tar_gz(source / "pg_wal.tar.gz", {"000000010000000000000009": b"wal"})

            with self.assertRaisesRegex(RestoreDrillError, "解不开"):
                materialize_base_backup(source, root / "restored")
            self.assertFalse((root / "escape").exists())


def inventory_line(path: str, content: bytes) -> bytes:
    """rclone lsjson --hash --hash-type sha256 列本地镜像时的一项。"""

    entry = {
        "Path": path,
        "Name": path.rsplit("/", 1)[-1],
        "Size": len(content),
        "ModTime": "2026-08-25T12:00:00.000000000Z",
        "IsDir": False,
        "Hashes": {"sha256": hashlib.sha256(content).hexdigest()},
    }
    return (json.dumps(entry) + "\n").encode()


def write_tar_gz(path: Path, members: dict[str, bytes | None]) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tarfile.open(path, "w:gz") as archive:
        for name, content in members.items():
            info = tarfile.TarInfo(name.rstrip("/"))
            if content is None:
                info.type = tarfile.DIRTYPE
                info.mode = 0o700
                archive.addfile(info)
                continue
            info.size = len(content)
            info.mode = 0o600
            archive.addfile(info, io.BytesIO(content))


def write(path: Path, content: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_bytes(content)


def store_segment(store: WalStore, name: str) -> None:
    """照 postgres-wal-archive.sh 收好一段：压缩的段，加一行 sha256sum 格式的记录。"""

    packed = b"wal " + name.encode()
    write(store.root / f"{name}.gz", packed)
    write(store.root / f"{name}.gz.sha256", f"{hashlib.sha256(packed).hexdigest()}  {name}.gz\n".encode())


if __name__ == "__main__":
    unittest.main()
