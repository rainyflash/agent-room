#!/usr/bin/env python3
"""从锁定的 Rust 与 Node 依赖生成可复现许可证清单。"""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import asdict, dataclass
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Final, Iterable
from urllib.parse import quote


ROOT: Final = Path(__file__).resolve().parents[1]
JSON_OUTPUT: Final = ROOT / "licenses" / "third-party.json"
MARKDOWN_OUTPUT: Final = ROOT / "THIRD_PARTY_NOTICES.md"


class LicenseInventoryError(RuntimeError):
    """表示依赖元数据不完整、冲突或生成物陈旧。"""


@dataclass(frozen=True, order=True, slots=True)
class DependencyLicense:
    ecosystem: str
    name: str
    version: str
    license: str
    source: str


def parse_cargo_metadata(value: object) -> tuple[DependencyLicense, ...]:
    root = _mapping(value, "Cargo metadata")
    packages = root.get("packages")
    if not isinstance(packages, list):
        raise LicenseInventoryError("Cargo metadata 缺少 packages 数组。")
    records: list[DependencyLicense] = []
    for package in packages:
        entry = _mapping(package, "Cargo package")
        if entry.get("source") is None:
            continue
        name = _text(entry, "name", "Cargo package")
        version = _text(entry, "version", f"Cargo package {name}")
        license_name = _license(entry.get("license"), f"Cargo package {name}@{version}")
        repository = entry.get("repository")
        source = (
            repository
            if isinstance(repository, str) and repository.startswith(("https://", "http://"))
            else f"https://crates.io/crates/{quote(name)}/{quote(version)}"
        )
        records.append(DependencyLicense("cargo", name, version, license_name, source))
    return _unique(records)


def parse_pnpm_licenses(
    value: object,
    *,
    excluded: Iterable[tuple[str, str]] = (),
) -> tuple[DependencyLicense, ...]:
    root = _mapping(value, "pnpm licenses")
    excluded_keys = frozenset(excluded)
    records: list[DependencyLicense] = []
    for grouped_license, packages in root.items():
        if not isinstance(packages, list):
            raise LicenseInventoryError(f"pnpm 许可证组 {grouped_license!r} 不是数组。")
        for package in packages:
            entry = _mapping(package, "pnpm package")
            name = _text(entry, "name", "pnpm package")
            if name == "agent-room" or name.startswith("@agent-room/"):
                continue
            versions = entry.get("versions")
            if not isinstance(versions, list) or not versions:
                raise LicenseInventoryError(f"pnpm package {name} 缺少 versions。")
            license_name = _license(entry.get("license", grouped_license), f"pnpm package {name}")
            for raw_version in versions:
                if not isinstance(raw_version, str) or not raw_version.strip():
                    raise LicenseInventoryError(f"pnpm package {name} 含无效版本。")
                version = raw_version.strip()
                if (name, version) in excluded_keys:
                    continue
                source = (
                    "https://www.npmjs.com/package/"
                    f"{quote(name, safe='@/')}/v/{quote(version, safe='.-')}"
                )
                records.append(DependencyLicense("npm", name, version, license_name, source))
    return _unique(records)


def platform_specific_pnpm_keys(
    value: object,
    *,
    workspace_root: Path = ROOT,
) -> frozenset[tuple[str, str]]:
    """识别仅服务某个 OS/CPU/libc 的可选二进制包。

    根级 notices 描述跨平台共有的源码依赖；平台二进制由各发行制品的 SBOM 记录。
    """

    root = _mapping(value, "pnpm licenses")
    node_modules = (workspace_root / "node_modules").resolve()
    constraints: dict[tuple[str, str], list[bool]] = {}
    for packages in root.values():
        if not isinstance(packages, list):
            raise LicenseInventoryError("pnpm 许可证组不是数组。")
        for package in packages:
            entry = _mapping(package, "pnpm package")
            name = _text(entry, "name", "pnpm package")
            if name == "agent-room" or name.startswith("@agent-room/"):
                continue
            paths = entry.get("paths")
            if not isinstance(paths, list) or not paths:
                raise LicenseInventoryError(f"pnpm package {name} 缺少 paths。")
            for raw_path in paths:
                if not isinstance(raw_path, str) or not raw_path:
                    raise LicenseInventoryError(f"pnpm package {name} 含无效路径。")
                package_path = Path(raw_path).resolve()
                if package_path != node_modules and node_modules not in package_path.parents:
                    raise LicenseInventoryError(f"pnpm package {name} 的路径越过 node_modules。")
                manifest_path = package_path / "package.json"
                try:
                    manifest = _mapping(
                        json.loads(manifest_path.read_text(encoding="utf-8")),
                        f"pnpm package {name} manifest",
                    )
                except json.JSONDecodeError as error:
                    raise LicenseInventoryError(
                        f"pnpm package {name} 的 package.json 无效。"
                    ) from error
                manifest_name = _text(manifest, "name", f"pnpm package {name} manifest")
                version = _text(manifest, "version", f"pnpm package {name} manifest")
                if manifest_name != name:
                    raise LicenseInventoryError(
                        f"pnpm package 路径声明 {manifest_name}，预期 {name}。"
                    )
                constrained = any(_has_platform_constraint(manifest.get(key)) for key in ("os", "cpu", "libc"))
                constraints.setdefault((name, version), []).append(constrained)
    return frozenset(key for key, values in constraints.items() if values and all(values))


def build_inventory(
    cargo: Iterable[DependencyLicense],
    npm: Iterable[DependencyLicense],
    *,
    cargo_lock_digest: str,
    pnpm_lock_digest: str,
) -> dict[str, object]:
    records = _unique((*cargo, *npm))
    return {
        "schemaVersion": 1,
        "inputs": {
            "Cargo.lock": cargo_lock_digest,
            "pnpm-lock.yaml": pnpm_lock_digest,
        },
        "dependencies": [asdict(record) for record in records],
    }


def render_markdown(inventory: dict[str, object]) -> str:
    dependencies = inventory.get("dependencies")
    if not isinstance(dependencies, list):
        raise LicenseInventoryError("许可证清单缺少 dependencies。")
    ecosystem_counts: Counter[str] = Counter()
    license_counts: Counter[str] = Counter()
    rows: list[str] = []
    for raw in dependencies:
        entry = _mapping(raw, "许可证记录")
        ecosystem = _text(entry, "ecosystem", "许可证记录")
        name = _text(entry, "name", "许可证记录")
        version = _text(entry, "version", "许可证记录")
        license_name = _text(entry, "license", "许可证记录")
        source = _text(entry, "source", "许可证记录")
        ecosystem_counts[ecosystem] += 1
        license_counts[license_name] += 1
        rows.append(
            f"| {_markdown(ecosystem)} | [{_markdown(name)}]({source}) | "
            f"{_markdown(version)} | `{_markdown(license_name)}` |"
        )

    summary = "\n".join(
        f"| `{_markdown(license_name)}` | {count} |"
        for license_name, count in sorted(license_counts.items(), key=lambda item: item[0].lower())
    )
    dependency_rows = "\n".join(rows)
    inputs = _mapping(inventory.get("inputs"), "inputs")
    return f"""# Third-party notices

This file is generated from the locked Rust and Node dependency graphs. Do not edit it by hand. Regenerate it with `python tools/license_inventory.py generate` and verify it with `python tools/license_inventory.py check`.

Agent Room source code is licensed under the MIT License. The dependencies below remain subject to their respective licenses. This inventory is informational and is not legal advice; distribution artifacts may include additional operating-system or container-image components covered by their own SBOMs.

## Inventory

- Cargo packages: {ecosystem_counts["cargo"]}
- npm packages: {ecosystem_counts["npm"]}
- Total locked package versions: {len(dependencies)}
- `Cargo.lock` SHA-256: `{_text(inputs, "Cargo.lock", "inputs")}`
- `pnpm-lock.yaml` SHA-256: `{_text(inputs, "pnpm-lock.yaml", "inputs")}`

## License expressions

| SPDX expression or declared license | Package versions |
| --- | ---: |
{summary}

## Packages

| Ecosystem | Package | Version | Declared license |
| --- | --- | --- | --- |
{dependency_rows}
"""


def collect_inventory() -> dict[str, object]:
    cargo_metadata = _run_json(["cargo", "metadata", "--format-version", "1", "--locked"])
    pnpm_licenses = _run_json(_pnpm_command())
    return build_inventory(
        parse_cargo_metadata(cargo_metadata),
        parse_pnpm_licenses(
            pnpm_licenses,
            excluded=platform_specific_pnpm_keys(pnpm_licenses),
        ),
        cargo_lock_digest=_digest(ROOT / "Cargo.lock"),
        pnpm_lock_digest=_digest(ROOT / "pnpm-lock.yaml"),
    )


def generate() -> None:
    inventory = collect_inventory()
    JSON_OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    JSON_OUTPUT.write_text(
        json.dumps(inventory, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    MARKDOWN_OUTPUT.write_text(render_markdown(inventory), encoding="utf-8", newline="\n")
    print(f"许可证清单已生成：{len(inventory['dependencies'])} 个锁定包版本。")


def check() -> None:
    inventory = collect_inventory()
    expected_json = json.dumps(inventory, indent=2, ensure_ascii=False) + "\n"
    expected_markdown = render_markdown(inventory)
    stale = [
        str(path.relative_to(ROOT))
        for path, expected in (
            (JSON_OUTPUT, expected_json),
            (MARKDOWN_OUTPUT, expected_markdown),
        )
        if not path.is_file() or path.read_text(encoding="utf-8") != expected
    ]
    if stale:
        raise LicenseInventoryError(
            "许可证生成物缺失或陈旧："
            + ", ".join(stale)
            + "。运行 python tools/license_inventory.py generate。"
        )
    print(f"许可证清单与锁文件一致：{len(inventory['dependencies'])} 个锁定包版本。")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("generate", "check"))
    arguments = parser.parse_args()
    try:
        if arguments.action == "generate":
            generate()
        else:
            check()
    except (LicenseInventoryError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


def _run_json(command: list[str]) -> object:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "无错误输出"
        raise LicenseInventoryError(f"依赖元数据命令失败：{detail}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise LicenseInventoryError("依赖元数据命令没有返回有效 JSON。") from error


def _pnpm_command() -> list[str]:
    package = _mapping(json.loads((ROOT / "package.json").read_text(encoding="utf-8")), "package.json")
    package_manager = _text(package, "packageManager", "package.json")
    if not package_manager.startswith("pnpm@"):
        raise LicenseInventoryError("packageManager 必须固定 pnpm 版本。")
    base = ["corepack", package_manager, "licenses", "list", "--json"]
    if os.name != "nt":
        return base
    return [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/s", "/c", " ".join(base)]


def _unique(records: Iterable[DependencyLicense]) -> tuple[DependencyLicense, ...]:
    indexed: dict[tuple[str, str, str], DependencyLicense] = {}
    for record in records:
        key = (record.ecosystem, record.name, record.version)
        existing = indexed.get(key)
        if existing is not None and existing != record:
            raise LicenseInventoryError(
                f"依赖许可证记录冲突：{record.ecosystem}:{record.name}@{record.version}。"
            )
        indexed[key] = record
    return tuple(sorted(indexed.values(), key=lambda item: (item.ecosystem, item.name.lower(), item.version)))


def _mapping(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise LicenseInventoryError(f"{label} 必须是对象。")
    return value


def _text(value: dict[str, object], key: str, label: str) -> str:
    result = value.get(key)
    if not isinstance(result, str) or not result.strip():
        raise LicenseInventoryError(f"{label}.{key} 必须是非空字符串。")
    return result.strip()


def _license(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise LicenseInventoryError(f"{label} 没有声明许可证。")
    return " ".join(value.split())


def _has_platform_constraint(value: object) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, list):
        return any(isinstance(item, str) and item.strip() for item in value)
    return False


def _digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _markdown(value: str) -> str:
    return value.replace("|", "\\|").replace("`", "\\`")


if __name__ == "__main__":
    raise SystemExit(main())
