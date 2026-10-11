"""一直留着的 WAL：恢复点记录的解析，和按最老的全量删旧段（specs/backups/design.md）。

`postgres-wal-archive.sh` 每次定时备份打一个恢复点，把接着上一个恢复点读通了的段压缩以后放进
`wal-store/`，再往 `restore-points.log` 追加一行。这里只读那份记录、按保留下来的全量删旧段；
段的内容由脚本在容器里核对。
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
import os
from pathlib import Path
import re
from typing import Final

from .restore_point import RESTORE_POINT_NAME, WAL_SEGMENT


WAL_STORE: Final = "wal-store"
RESTORE_POINT_LOG: Final = "restore-points.log"
# 每行：时间 种类 恢复点名字 LSN 到哪个段为止 全量 ID（没有就是 -）。种类 anchor 是接起的那套全量
# 的恢复点，point 是之后每次定时打的恢复点；一份记录就是一条接得上的链，第一行是起点。
_POINT: Final = re.compile(
    r"^(?P<created_at>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z) "
    r"(?P<kind>anchor|point) (?P<name>\S+) (?P<lsn>[0-9A-F]{1,8}/[0-9A-F]{1,8}) "
    r"(?P<segment>[0-9A-F]{24}) (?P<backup_id>-|[0-9]{8}T[0-9]{12}Z-[0-9a-f]{8})$"
)
_STORED: Final = re.compile(r"^(?P<segment>[0-9A-F]{24})\.gz(?:\.sha256)?$")


class WalStoreError(RuntimeError):
    """表示一直留着的 WAL 的记录损坏或者对不上。"""


@dataclass(frozen=True, slots=True)
class WalAnchor:
    """还没有恢复点记录时，从这套全量的恢复点接起。"""

    backup_id: str
    name: str
    lsn: str
    segment: str

    @property
    def environment_value(self) -> str:
        return f"{self.backup_id} {self.name} {self.lsn} {self.segment}"


@dataclass(frozen=True, slots=True)
class WalPoint:
    created_at: datetime
    kind: str
    name: str
    lsn: str
    segment: str
    backup_id: str | None

    @classmethod
    def parse(cls, line: str) -> "WalPoint":
        match = _POINT.fullmatch(line)
        if match is None or not RESTORE_POINT_NAME.fullmatch(match["name"]):
            raise WalStoreError("恢复点记录里有一行格式不对。")
        created_at = datetime.strptime(match["created_at"], "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=UTC)
        backup_id = None if match["backup_id"] == "-" else match["backup_id"]
        if (match["kind"] == "anchor") != (backup_id is not None):
            raise WalStoreError("恢复点记录里只有接起的全量带全量 ID。")
        return cls(created_at, match["kind"], match["name"], match["lsn"], match["segment"], backup_id)

    def to_line(self) -> str:
        created_at = self.created_at.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%S.%fZ")
        return " ".join(
            (created_at, self.kind, self.name, self.lsn, self.segment, self.backup_id or "-")
        )


@dataclass(frozen=True, slots=True)
class WalStore:
    root: Path

    @property
    def log(self) -> Path:
        return self.root / RESTORE_POINT_LOG

    def points(self) -> tuple[WalPoint, ...]:
        if not self.log.exists():
            return ()
        if self.log.is_symlink() or not self.log.is_file():
            raise WalStoreError("恢复点记录不是常规文件。")
        try:
            text = self.log.read_text(encoding="utf-8")
        except UnicodeDecodeError as error:
            raise WalStoreError("恢复点记录不是 UTF-8。") from error
        points = tuple(WalPoint.parse(line) for line in text.splitlines())
        for previous, current in zip(points, points[1:]):
            if current.segment < previous.segment or current.created_at < previous.created_at:
                raise WalStoreError("恢复点记录没有按时间往前走。")
        return points

    def latest(self) -> WalPoint | None:
        points = self.points()
        return points[-1] if points else None

    def segments(self) -> tuple[str, ...]:
        if not self.root.is_dir():
            return ()
        names = (path.name for path in self.root.iterdir() if path.name.endswith(".gz"))
        return tuple(sorted(name.removesuffix(".gz") for name in names if _STORED.fullmatch(name)))

    def prune(self, keep_from: str) -> tuple[str, ...]:
        """删掉 `keep_from` 以前的段和恢复点记录。

        `keep_from` 是保留下来的最老那套全量的起始段：从它恢复要用到它以后的全部 WAL。最后两个恢复点
        和它们之间的段一定留着，下一次归档要从最后一个恢复点所在的段接着读。
        """

        if not WAL_SEGMENT.fullmatch(keep_from):
            raise WalStoreError("保留起点不是 WAL 段名。")
        points = self.points()
        if len(points) >= 2:
            keep_from = min(keep_from, points[-2].segment)
        removed: list[str] = []
        if self.root.is_dir():
            for path in sorted(self.root.iterdir(), key=lambda item: item.name):
                match = _STORED.fullmatch(path.name)
                if match is None or match["segment"] >= keep_from:
                    continue
                if path.is_symlink() or not path.is_file():
                    raise WalStoreError(f"WAL 仓库里有不安全的条目：{path.name}。")
                path.unlink(missing_ok=True)
                if path.name.endswith(".gz"):
                    removed.append(match["segment"])
        kept = [point for point in points[:-2] if point.segment >= keep_from] + list(points[-2:])
        if len(kept) != len(points):
            self._write(tuple(kept))
        return tuple(removed)

    def _write(self, points: tuple[WalPoint, ...]) -> None:
        temporary = self.root / f".{RESTORE_POINT_LOG}.partial"
        temporary.write_text(
            "".join(point.to_line() + "\n" for point in points), encoding="utf-8", newline="\n"
        )
        temporary.chmod(0o600)
        os.replace(temporary, self.log)
