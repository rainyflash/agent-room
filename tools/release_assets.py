#!/usr/bin/env python3
"""收集、证明并描述 Agent Room 原生发布产物。"""

from __future__ import annotations

import argparse
from datetime import UTC, datetime
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Final, Mapping, Sequence
from urllib.parse import quote

if __package__:
    from .release import ReleaseFailure, create_descriptor, parse_args as parse_release_args
else:
    from release import ReleaseFailure, create_descriptor, parse_args as parse_release_args


TARGETS: Final = {
    "x86_64-pc-windows-msvc": ("windows-x86_64", ("*.nsis.zip", "*.msi.zip")),
    "aarch64-pc-windows-msvc": ("windows-aarch64", ("*.nsis.zip", "*.msi.zip")),
    "x86_64-apple-darwin": ("darwin-x86_64", ("*.app.tar.gz",)),
    "aarch64-apple-darwin": ("darwin-aarch64", ("*.app.tar.gz",)),
    "x86_64-unknown-linux-gnu": ("linux-x86_64", ("*.AppImage.tar.gz",)),
    "aarch64-unknown-linux-gnu": ("linux-aarch64", ("*.AppImage.tar.gz",)),
}


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    collect_parser = subcommands.add_parser("collect-native", help="收集当前平台发布文件")
    collect_parser.add_argument("--bundle-root", type=Path, required=True)
    collect_parser.add_argument("--bridge", type=Path, required=True)
    collect_parser.add_argument("--plugin", type=Path, required=True)
    collect_parser.add_argument("--output-root", type=Path, required=True)
    collect_parser.add_argument("--metadata", type=Path, required=True)
    collect_parser.add_argument("--release-base-url", required=True)
    collect_parser.add_argument("--version", required=True)
    collect_parser.add_argument("--rust-target")

    attest_parser = subcommands.add_parser("attest-blobs", help="为原生文件生成 SBOM 与签名")
    attest_parser.add_argument("--root", type=Path, required=True)
    attest_parser.add_argument("--metadata", type=Path, required=True)
    attest_parser.add_argument("--descriptor-root", type=Path, required=True)
    attest_parser.add_argument("--syft", default="syft")
    attest_parser.add_argument("--cosign", default="cosign")

    blob_parser = subcommands.add_parser("attest-blob", help="证明并描述一个普通发布文件")
    blob_parser.add_argument("--root", type=Path, required=True)
    blob_parser.add_argument("--path", required=True)
    blob_parser.add_argument("--name", required=True)
    blob_parser.add_argument(
        "--kind",
        choices=("bridge", "desktop", "codex-plugin", "update-manifest"),
        required=True,
    )
    blob_parser.add_argument("--platform", required=True)
    blob_parser.add_argument("--url", required=True)
    blob_parser.add_argument("--descriptor", type=Path, required=True)
    blob_parser.add_argument("--syft", default="syft")
    blob_parser.add_argument("--cosign", default="cosign")

    image_parser = subcommands.add_parser("attest-image", help="证明并描述一个 OCI 镜像")
    image_parser.add_argument("--root", type=Path, required=True)
    image_parser.add_argument("--manifest-path", required=True)
    image_parser.add_argument("--image-ref", required=True)
    image_parser.add_argument("--name", required=True)
    image_parser.add_argument("--platform", required=True)
    image_parser.add_argument("--descriptor", type=Path, required=True)
    image_parser.add_argument("--release-base-url", required=True)
    image_parser.add_argument("--syft", default="syft")
    image_parser.add_argument("--cosign", default="cosign")

    merge_parser = subcommands.add_parser("merge-tauri", help="合并各平台 Tauri 更新条目")
    merge_parser.add_argument("--metadata-root", type=Path, required=True)
    merge_parser.add_argument("--version", required=True)
    merge_parser.add_argument("--published-at-unix-seconds", type=int, required=True)
    merge_parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def run_capture(command: Sequence[str], label: str) -> str:
    try:
        completed = subprocess.run(
            command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
    except OSError as error:
        raise ReleaseFailure(f"{label}无法启动：{command[0]}") from error
    if completed.returncode != 0:
        raise ReleaseFailure(
            f"{label}失败（退出码 {completed.returncode}）：\n{completed.stdout.strip()}"
        )
    return completed.stdout


def detect_rust_target() -> str:
    output = run_capture(("rustc", "-vV"), "读取 Rust 目标")
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    raise ReleaseFailure("rustc -vV 没有返回 host 目标。")


def require_file(path: Path, label: str) -> Path:
    resolved = path.resolve(strict=True)
    if not resolved.is_file() or resolved.stat().st_size == 0:
        raise ReleaseFailure(f"{label}必须是非空普通文件：{path}")
    return resolved


def find_updater_archive(bundle_root: Path, patterns: Sequence[str]) -> Path:
    root = bundle_root.resolve(strict=True)
    for pattern in patterns:
        matches = sorted(root.rglob(pattern))
        if len(matches) == 1:
            return require_file(matches[0], "Tauri 更新归档")
        if len(matches) > 1:
            raise ReleaseFailure(f"Tauri 更新归档不唯一（{pattern}）：{len(matches)} 个")
    raise ReleaseFailure(f"没有找到 Tauri 更新归档：{', '.join(patterns)}")


def compound_suffix(path: Path) -> str:
    name = path.name
    for suffix in (".app.tar.gz", ".AppImage.tar.gz", ".nsis.zip", ".msi.zip"):
        if name.endswith(suffix):
            return suffix
    raise ReleaseFailure(f"不支持的 Tauri 更新归档：{path}")


def release_url(base: str, filename: str) -> str:
    if not base.startswith("https://"):
        raise ReleaseFailure("发布资产基地址必须使用 HTTPS。")
    return f"{base.rstrip('/')}/{quote(filename)}"


def copy_new(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise ReleaseFailure(f"拒绝覆盖已有发布产物：{destination}")
    shutil.copy2(source, destination)


def write_new_json(path: Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(value, target, ensure_ascii=False, indent=2)
            target.write("\n")
    except FileExistsError as error:
        raise ReleaseFailure(f"拒绝覆盖已有元数据：{path}") from error


def collect_native(args: argparse.Namespace) -> None:
    rust_target = args.rust_target or detect_rust_target()
    target = TARGETS.get(rust_target)
    if target is None:
        raise ReleaseFailure(f"不支持的 Rust 发布目标：{rust_target}")
    updater_target, patterns = target
    archive = find_updater_archive(args.bundle_root, patterns)
    tauri_signature = require_file(Path(f"{archive}.sig"), "Tauri 更新签名")
    bridge = require_file(args.bridge, "Bridge")
    plugin = require_file(args.plugin, "Codex 插件")
    output_root = args.output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)

    desktop_name = f"agent-room-desktop-v{args.version}-{updater_target}{compound_suffix(archive)}"
    bridge_suffix = ".exe" if updater_target.startswith("windows-") else ""
    bridge_name = f"agent-room-bridge-v{args.version}-{updater_target}{bridge_suffix}"
    plugin_name = f"agent-room-codex-plugin-v{args.version}-{updater_target}.zip"
    desktop_path = output_root / desktop_name
    desktop_signature_path = output_root / f"{desktop_name}.sig"
    bridge_path = output_root / bridge_name
    plugin_path = output_root / plugin_name
    copy_new(archive, desktop_path)
    copy_new(tauri_signature, desktop_signature_path)
    copy_new(bridge, bridge_path)
    copy_new(plugin, plugin_path)

    metadata = {
        "schemaVersion": 1,
        "rustTarget": rust_target,
        "updaterTarget": updater_target,
        "tauriSignaturePath": desktop_signature_path.relative_to(output_root).as_posix(),
        "artifacts": [
            {
                "name": "desktop",
                "kind": "desktop",
                "platform": updater_target,
                "path": desktop_path.relative_to(output_root).as_posix(),
                "url": release_url(args.release_base_url, desktop_name),
            },
            {
                "name": "bridge",
                "kind": "bridge",
                "platform": updater_target,
                "path": bridge_path.relative_to(output_root).as_posix(),
                "url": release_url(args.release_base_url, bridge_name),
            },
            {
                "name": "codex-plugin",
                "kind": "codex-plugin",
                "platform": updater_target,
                "path": plugin_path.relative_to(output_root).as_posix(),
                "url": release_url(args.release_base_url, plugin_name),
            },
        ],
    }
    write_new_json(args.metadata, metadata)


def load_object(path: Path, label: str) -> Mapping[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ReleaseFailure(f"{label}不是有效 JSON：{path}") from error
    if not isinstance(value, dict):
        raise ReleaseFailure(f"{label}必须是 JSON 对象：{path}")
    return value


def metadata_artifacts(metadata: Mapping[str, object]) -> tuple[Mapping[str, object], ...]:
    value = metadata.get("artifacts")
    if not isinstance(value, list) or not value:
        raise ReleaseFailure("原生产物元数据缺少 artifacts。")
    artifacts: list[Mapping[str, object]] = []
    for item in value:
        if not isinstance(item, dict):
            raise ReleaseFailure("原生产物条目必须是对象。")
        artifacts.append(item)
    return tuple(artifacts)


def string_field(source: Mapping[str, object], key: str, label: str) -> str:
    value = source.get(key)
    if not isinstance(value, str) or not value:
        raise ReleaseFailure(f"{label}.{key} 必须是非空字符串。")
    return value


def attest_blobs(args: argparse.Namespace) -> None:
    root = args.root.resolve(strict=True)
    metadata = load_object(args.metadata, "原生产物元数据")
    args.descriptor_root.mkdir(parents=True, exist_ok=True)
    for artifact in metadata_artifacts(metadata):
        name = string_field(artifact, "name", "artifact")
        kind = string_field(artifact, "kind", "artifact")
        platform = string_field(artifact, "platform", "artifact")
        relative = Path(string_field(artifact, "path", "artifact"))
        path = require_file(root / relative, "原生产物")
        sbom = path.with_name(f"{path.name}.cdx.json")
        signature = path.with_name(f"{path.name}.sigstore.json")
        run_capture(
            (args.syft, "scan", str(path), "-o", f"cyclonedx-json={sbom}"),
            f"生成 {name} SBOM",
        )
        run_capture(
            (
                args.cosign,
                "sign-blob",
                str(path),
                "--bundle",
                str(signature),
                "--yes",
            ),
            f"签名 {name}",
        )
        url = string_field(artifact, "url", "artifact")
        descriptor_args = parse_release_args(
            [
                "descriptor",
                "--root",
                str(root),
                "--output",
                str(args.descriptor_root / f"{kind}-{platform}-{name}.artifact.json"),
                "--name",
                name,
                "--kind",
                kind,
                "--platform",
                platform,
                "--path",
                path.relative_to(root).as_posix(),
                "--url",
                url,
                "--sbom-path",
                sbom.relative_to(root).as_posix(),
                "--sbom-url",
                f"{url}.cdx.json",
                "--signature-path",
                signature.relative_to(root).as_posix(),
                "--signature-url",
                f"{url}.sigstore.json",
                "--signature-mode",
                "blob",
            ]
        )
        create_descriptor(descriptor_args)


def attest_blob(args: argparse.Namespace) -> None:
    root = args.root.resolve(strict=True)
    path = require_file(root / Path(args.path), "发布文件")
    sbom = path.with_name(f"{path.name}.cdx.json")
    signature = path.with_name(f"{path.name}.sigstore.json")
    run_capture(
        (args.syft, "scan", str(path), "-o", f"cyclonedx-json={sbom}"),
        f"生成 {args.name} SBOM",
    )
    run_capture(
        (args.cosign, "sign-blob", str(path), "--bundle", str(signature), "--yes"),
        f"签名 {args.name}",
    )
    descriptor_args = parse_release_args(
        [
            "descriptor",
            "--root",
            str(root),
            "--output",
            str(args.descriptor),
            "--name",
            args.name,
            "--kind",
            args.kind,
            "--platform",
            args.platform,
            "--path",
            path.relative_to(root).as_posix(),
            "--url",
            args.url,
            "--sbom-path",
            sbom.relative_to(root).as_posix(),
            "--sbom-url",
            f"{args.url}.cdx.json",
            "--signature-path",
            signature.relative_to(root).as_posix(),
            "--signature-url",
            f"{args.url}.sigstore.json",
            "--signature-mode",
            "blob",
        ]
    )
    create_descriptor(descriptor_args)


def attest_image(args: argparse.Namespace) -> None:
    root = args.root.resolve(strict=True)
    manifest = require_file(root / Path(args.manifest_path), "OCI manifest")
    digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
    expected_suffix = f"@sha256:{digest}"
    if not args.image_ref.endswith(expected_suffix):
        raise ReleaseFailure("OCI manifest 摘要与不可变镜像引用不一致。")
    sbom = manifest.with_name(f"{manifest.name}.cdx.json")
    signature = manifest.with_name(f"{manifest.name}.sigstore.json")
    run_capture(
        (args.syft, "scan", args.image_ref, "-o", f"cyclonedx-json={sbom}"),
        f"生成 {args.name} 镜像 SBOM",
    )
    run_capture(
        (args.cosign, "sign", args.image_ref, "--bundle", str(signature), "--yes"),
        f"签名 {args.name} 镜像",
    )
    manifest_url = release_url(args.release_base_url, manifest.name)
    descriptor_args = parse_release_args(
        [
            "descriptor",
            "--root",
            str(root),
            "--output",
            str(args.descriptor),
            "--name",
            args.name,
            "--kind",
            "oci-image",
            "--platform",
            args.platform,
            "--path",
            manifest.relative_to(root).as_posix(),
            "--url",
            f"oci://{args.image_ref}",
            "--sbom-path",
            sbom.relative_to(root).as_posix(),
            "--sbom-url",
            f"{manifest_url}.cdx.json",
            "--signature-path",
            signature.relative_to(root).as_posix(),
            "--signature-url",
            f"{manifest_url}.sigstore.json",
            "--signature-mode",
            "oci",
        ]
    )
    create_descriptor(descriptor_args)


def merge_tauri(args: argparse.Namespace) -> None:
    if args.published_at_unix_seconds < 0:
        raise ReleaseFailure("Tauri 发布时间不能为负数。")
    metadata_paths = sorted(
        args.metadata_root.resolve(strict=True).rglob("native-metadata-*.json")
    )
    if not metadata_paths:
        raise ReleaseFailure("没有找到原生平台元数据。")
    platforms: dict[str, object] = {}
    for metadata_path in metadata_paths:
        metadata = load_object(metadata_path, "原生平台元数据")
        target = string_field(metadata, "updaterTarget", "metadata")
        if target in platforms:
            raise ReleaseFailure(f"Tauri 平台重复：{target}")
        desktop = [
            item
            for item in metadata_artifacts(metadata)
            if item.get("kind") == "desktop"
        ]
        if len(desktop) != 1:
            raise ReleaseFailure(f"平台 {target} 必须且只能有一个桌面更新产物。")
        signature_relative = string_field(metadata, "tauriSignaturePath", "metadata")
        signature = require_file(metadata_path.parent / signature_relative, "Tauri 更新签名")
        platforms[target] = {
            "signature": signature.read_text(encoding="utf-8").strip(),
            "url": string_field(desktop[0], "url", "desktop"),
        }
    published = datetime.fromtimestamp(args.published_at_unix_seconds, UTC).isoformat().replace(
        "+00:00", "Z"
    )
    write_new_json(
        args.output,
        {
            "version": args.version,
            "notes": "See the signed Agent Room release notes.",
            "pub_date": published,
            "platforms": platforms,
        },
    )


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        if args.command == "collect-native":
            collect_native(args)
        elif args.command == "attest-blobs":
            attest_blobs(args)
        elif args.command == "attest-blob":
            attest_blob(args)
        elif args.command == "attest-image":
            attest_image(args)
        else:
            merge_tauri(args)
        return 0
    except ReleaseFailure as error:
        print(f"发布资产失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
