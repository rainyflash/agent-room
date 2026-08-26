#!/usr/bin/env python3
"""验证 Agent Room 开源发行面的文档、链接与示例数据。"""

from __future__ import annotations

import json
from pathlib import Path
import re
import sys
from typing import Final, Iterable
from urllib.parse import unquote, urlparse


ROOT: Final = Path(__file__).resolve().parents[1]
REQUIRED_FILES: Final = (
    "README.md",
    "README.zh-CN.md",
    "CONTRIBUTING.md",
    "CODE_OF_CONDUCT.md",
    "SECURITY.md",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "licenses/third-party.json",
    "docs/architecture.md",
    "docs/self-hosting.md",
    "docs/compatibility.md",
    "docs/known-limitations.md",
    "docs/adr/README.md",
)
PUBLIC_DOCUMENTS: Final = (
    "README.md",
    "README.zh-CN.md",
    "CONTRIBUTING.md",
    "CODE_OF_CONDUCT.md",
    "SECURITY.md",
    "THIRD_PARTY_NOTICES.md",
)
MARKDOWN_LINK: Final = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
PLACEHOLDER: Final = re.compile(r"\b(?:TODO|TBD|FIXME|example\.org)\b", re.IGNORECASE)
RESERVED_SUFFIXES: Final = (".example", ".example.com", ".test", ".invalid", ".localhost")


class OpenSourceValidationError(RuntimeError):
    """表示开源发行面包含断链、占位符或真实服务示例。"""


def validate_required_files(root: Path = ROOT) -> None:
    missing = [relative for relative in REQUIRED_FILES if not root.joinpath(relative).is_file()]
    if missing:
        raise OpenSourceValidationError("缺少开源发行文件：" + ", ".join(missing))


def validate_markdown_links(paths: Iterable[Path], root: Path = ROOT) -> None:
    failures: list[str] = []
    for path in paths:
        text = path.read_text(encoding="utf-8")
        for raw_target in MARKDOWN_LINK.findall(text):
            target = raw_target.strip().split(maxsplit=1)[0].strip("<>")
            if target.startswith(("#", "http://", "https://", "mailto:")):
                continue
            relative = unquote(target.split("#", 1)[0])
            if not relative:
                continue
            resolved = (path.parent / relative).resolve()
            try:
                resolved.relative_to(root.resolve())
            except ValueError:
                failures.append(f"{path.relative_to(root)} -> {target}（越出仓库）")
                continue
            if not resolved.exists():
                failures.append(f"{path.relative_to(root)} -> {target}")
    if failures:
        raise OpenSourceValidationError("Markdown 本地链接无效：\n" + "\n".join(failures))


def validate_no_placeholders(paths: Iterable[Path], root: Path = ROOT) -> None:
    failures: list[str] = []
    for path in paths:
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if PLACEHOLDER.search(line):
                failures.append(f"{path.relative_to(root)}:{line_number}")
    if failures:
        raise OpenSourceValidationError("公开文档仍含占位标记：" + ", ".join(failures))


def validate_reserved_example(path: Path) -> None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise OpenSourceValidationError(f"示例不是有效 JSON：{path}") from error
    failures: list[str] = []

    def visit(node: object, key_path: str) -> None:
        if isinstance(node, dict):
            for key, child in node.items():
                visit(child, f"{key_path}.{key}" if key_path else key)
            return
        if isinstance(node, list):
            for index, child in enumerate(node):
                visit(child, f"{key_path}[{index}]")
            return
        if not isinstance(node, str):
            return
        key = key_path.rsplit(".", 1)[-1].lower()
        if key == "acmeemail":
            domain = node.rsplit("@", 1)[-1].lower()
            if domain != "example.com":
                failures.append(f"{key_path}={node}")
            return
        if key in {"servername", "appdomain", "apidomain", "matrixdomain", "identitydomain", "host"}:
            if not _reserved_host(node):
                failures.append(f"{key_path}={node}")
            return
        if key in {"endpoint", "healthurl", "alertwebhookurl", "$schema"}:
            parsed = urlparse(node)
            if parsed.hostname is not None and not _reserved_host(parsed.hostname):
                failures.append(f"{key_path}={node}")
        if any(marker in key for marker in ("password", "secret", "token", "privatekey")):
            failures.append(f"{key_path} 不得出现在公开示例中")

    visit(value, "")
    if failures:
        raise OpenSourceValidationError(
            f"示例必须只使用保留域名和无凭据字段：{_display_path(path)}："
            + ", ".join(failures)
        )


def validate_license_inventory(path: Path = ROOT / "licenses" / "third-party.json") -> None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError) as error:
        raise OpenSourceValidationError("许可证 JSON 缺失或无效。") from error
    serialized = json.dumps(value, ensure_ascii=False).lower()
    forbidden = ("c:\\users\\", "/users/", "/home/", "node_modules/.pnpm")
    if any(marker in serialized for marker in forbidden):
        raise OpenSourceValidationError("许可证清单泄露了本地路径。")
    dependencies = value.get("dependencies") if isinstance(value, dict) else None
    if not isinstance(dependencies, list) or not dependencies:
        raise OpenSourceValidationError("许可证清单没有依赖记录。")


def public_markdown_paths(root: Path = ROOT) -> tuple[Path, ...]:
    explicit = [root / relative for relative in PUBLIC_DOCUMENTS]
    docs = sorted(root.joinpath("docs").rglob("*.md"))
    production = [root / "infra" / "production" / "README.md"]
    return tuple(explicit + docs + production)


def validate() -> None:
    validate_required_files()
    documents = public_markdown_paths()
    validate_markdown_links(documents)
    validate_no_placeholders(documents)
    for path in sorted((ROOT / "infra" / "production").glob("*example*.json")):
        validate_reserved_example(path)
    validate_license_inventory()
    print(
        f"开源发行面验证通过：{len(documents)} 个文档、"
        f"{len(tuple((ROOT / 'infra' / 'production').glob('*example*.json')))} 个配置示例。"
    )


def _reserved_host(value: str) -> bool:
    host = value.lower().rstrip(".")
    return host == "localhost" or host.endswith(RESERVED_SUFFIXES)


def _display_path(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT.resolve()))
    except ValueError:
        return str(path)


def main() -> int:
    try:
        validate()
    except OpenSourceValidationError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
