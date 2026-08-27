"""PostgreSQL 恢复点元数据的领域模型与严格解析。"""

from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import re
from typing import Final


RESTORE_POINT_NAME: Final = re.compile(r"^[A-Za-z0-9_]{1,200}$")
LSN: Final = re.compile(r"^[0-9A-F]+/[0-9A-F]+$")
WAL_SEGMENT: Final = re.compile(r"^[0-9A-F]{24}$")


class RestorePointError(ValueError):
    """表示恢复点元数据缺失、损坏或不符合恢复契约。"""


@dataclass(frozen=True, slots=True)
class RestorePoint:
    name: str
    lsn: str
    last_required_wal: str

    @classmethod
    def load(cls, path: Path) -> "RestorePoint":
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (FileNotFoundError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise RestorePointError("恢复点元数据缺失或损坏。") from error
        if not isinstance(value, dict) or set(value) != {"name", "lsn", "lastRequiredWal"}:
            raise RestorePointError("恢复点元数据字段无效。")

        name = value.get("name")
        lsn = value.get("lsn")
        wal = value.get("lastRequiredWal")
        if not isinstance(name, str) or not RESTORE_POINT_NAME.fullmatch(name):
            raise RestorePointError("恢复点名称无效。")
        if not isinstance(lsn, str) or not LSN.fullmatch(lsn):
            raise RestorePointError("恢复点 LSN 无效。")
        if not isinstance(wal, str) or not WAL_SEGMENT.fullmatch(wal):
            raise RestorePointError("恢复点 WAL 文件名无效。")
        return cls(name=name, lsn=lsn, last_required_wal=wal)
