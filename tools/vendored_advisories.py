#!/usr/bin/env python3
"""看仓库里自带的第三方 crate 有没有还没看过的安全公告。

cargo-deny 只拿 crates.io 来的包对安全公告，`[patch.crates-io]` 换成路径依赖的包它不查：
2026-11-13 vodozemac 换成 third_party 里打过补丁的 0.10.0 以后，它就把 RUSTSEC-2026-0451
报成“没遇到”。所以这个检查接在 `cargo deny check` 后面，读它拉下来的 RustSec 公告库，
third_party/vendored.toml 里的 crate 只要有一条没记过的公告就失败，看过以后再记上。
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import sys
import tomllib
from typing import Final, Sequence


ROOT: Final = Path(__file__).resolve().parent.parent
VENDORED: Final = ROOT / "third_party" / "vendored.toml"
STATUSES: Final = frozenset({"patched", "unaffected"})
FENCE_START: Final = "```toml\n"
FENCE_END: Final = "\n```"


class VendoredAdvisoryError(RuntimeError):
    """vendored.toml 写得不对，或者读不到公告库。"""


@dataclass(frozen=True)
class Advisory:
    id: str
    title: str
    url: str


@dataclass(frozen=True)
class VendoredCrate:
    name: str
    version: str
    reviewed: frozenset[str]


def load_vendored(path: Path = VENDORED, root: Path = ROOT) -> tuple[VendoredCrate, ...]:
    """读出自带的 crate 和看过的公告，顺带核对 crate 自己的 Cargo.toml 还是记着的那个版本。"""

    data = tomllib.loads(path.read_text(encoding="utf-8"))
    entries = data.get("crate")
    if not isinstance(entries, list) or not entries:
        raise VendoredAdvisoryError(f"{path.name} 里没有 [[crate]]。")
    crates: list[VendoredCrate] = []
    for entry in entries:
        name = _text(entry, "name", path.name)
        version = _text(entry, "version", name)
        manifest = root / _text(entry, "path", name) / "Cargo.toml"
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        if package.get("name") != name or package.get("version") != version:
            raise VendoredAdvisoryError(
                f"{manifest} 写的是 {package.get('name')} {package.get('version')}，"
                f"和 {path.name} 记的 {name} {version} 对不上：换了版本要把公告重新看一遍。"
            )
        crates.append(VendoredCrate(name, version, _reviewed(entry, name)))
    return tuple(crates)


def _reviewed(entry: object, crate: str) -> frozenset[str]:
    advisories = entry.get("advisory", []) if isinstance(entry, dict) else None
    if not isinstance(advisories, list):
        raise VendoredAdvisoryError(f"{crate} 的 advisory 要写成 [[crate.advisory]]。")
    reviewed: set[str] = set()
    for advisory in advisories:
        advisory_id = _text(advisory, "id", crate)
        status = _text(advisory, "status", advisory_id)
        if status not in STATUSES:
            raise VendoredAdvisoryError(f"{advisory_id} 的 status 只能是 patched 或 unaffected，写的是 {status}。")
        _text(advisory, "note", advisory_id)
        if advisory_id in reviewed:
            raise VendoredAdvisoryError(f"{crate} 里 {advisory_id} 记了两次。")
        reviewed.add(advisory_id)
    return frozenset(reviewed)


def _text(entry: object, key: str, context: str) -> str:
    value = entry.get(key) if isinstance(entry, dict) else None
    if not isinstance(value, str) or not value.strip():
        raise VendoredAdvisoryError(f"{context} 缺少 {key}。")
    return value


def default_databases() -> tuple[Path, ...]:
    """cargo-deny 把公告库拉到 $CARGO_HOME/advisory-dbs/advisory-db-<哈希>。"""

    cargo_home = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")
    return tuple(sorted((cargo_home / "advisory-dbs").glob("advisory-db-*")))


def database_advisories(database: Path, crate: str) -> tuple[Advisory, ...]:
    """读出公告库里这个 crate 的公告，撤回了的不算。"""

    if not (database / "crates").is_dir():
        raise VendoredAdvisoryError(f"{database} 不像 RustSec 公告库（没有 crates 目录）。")
    found: list[Advisory] = []
    for path in sorted((database / "crates" / crate).glob("RUSTSEC-*.md")):
        text = path.read_text(encoding="utf-8").replace("\r\n", "\n")
        start = text.find(FENCE_START)
        end = text.find(FENCE_END, start + len(FENCE_START))
        if start < 0 or end < 0:
            raise VendoredAdvisoryError(f"{path} 开头没有 toml 代码块。")
        advisory = tomllib.loads(text[start + len(FENCE_START) : end]).get("advisory", {})
        if advisory.get("withdrawn"):
            continue
        title = next((line[2:].strip() for line in text[end:].splitlines() if line.startswith("# ")), "")
        found.append(Advisory(_text(advisory, "id", str(path)), title, str(advisory.get("url", ""))))
    return tuple(found)


def unreviewed(crates: Sequence[VendoredCrate], databases: Sequence[Path]) -> list[tuple[VendoredCrate, Advisory]]:
    if not databases:
        raise VendoredAdvisoryError("找不到 RustSec 公告库：先跑一次 `cargo deny check`，它会把公告库拉下来。")
    missing: dict[tuple[str, str], tuple[VendoredCrate, Advisory]] = {}
    for crate in crates:
        for database in databases:
            for advisory in database_advisories(database, crate.name):
                if advisory.id not in crate.reviewed:
                    missing[(crate.name, advisory.id)] = (crate, advisory)
    return [missing[key] for key in sorted(missing)]


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--database", type=Path, action="append", help="RustSec 公告库目录，不给就找 cargo-deny 拉下来的")
    args = parser.parse_args(argv)
    try:
        crates = load_vendored()
        found = unreviewed(crates, tuple(args.database) if args.database else default_databases())
    except (OSError, tomllib.TOMLDecodeError, VendoredAdvisoryError) as error:
        print(f"自带 crate 的公告检查出错：{error}", file=sys.stderr)
        return 2
    for crate, advisory in found:
        print(f"{crate.name} {crate.version} 有没看过的公告 {advisory.id}：{advisory.title} {advisory.url}".rstrip(), file=sys.stderr)
    if found:
        print("看它影不影响自带的这一份，补上或确认不受影响以后记进 third_party/vendored.toml。", file=sys.stderr)
        return 1
    names = "、".join(f"{crate.name} {crate.version}" for crate in crates)
    print(f"自带的 {names} 没有没看过的公告。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
