#!/usr/bin/env python3
"""准备并验证 Agent Room 的签名发布候选。"""

from __future__ import annotations

import argparse
import base64
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Final, Mapping, Sequence
from urllib.parse import urlparse


ROOT: Final = Path(__file__).resolve().parent.parent
SCHEMA_VERSION: Final = 1
VALID_CHANNELS: Final = frozenset({"stable", "testing"})
VALID_KINDS: Final = frozenset(
    {"oci-image", "bridge", "desktop", "codex-plugin", "update-manifest"}
)
REQUIRED_KINDS: Final = VALID_KINDS
REQUIRED_OCI_IMAGES: Final = frozenset({"control-plane", "identity", "web"})
NAME_PATTERN: Final = re.compile(r"^[a-z0-9][a-z0-9.-]{0,63}$")
PLATFORM_PATTERN: Final = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")
SHA256_PATTERN: Final = re.compile(r"^[0-9a-f]{64}$")
SIGSTORE_MEDIA_TYPE_PREFIX: Final = "application/vnd.dev.sigstore.bundle."


class ReleaseFailure(RuntimeError):
    """表示发布候选无法被安全构建或验证。"""


@dataclass(frozen=True)
class ArtifactSource:
    """描述一个本地产物及其远端发布元数据。"""

    name: str
    kind: str
    platform: str
    path: Path
    url: str
    sbom_path: Path
    sbom_url: str
    signature_path: Path
    signature_url: str
    signature_mode: str


@dataclass(frozen=True)
class CandidatePaths:
    """封装候选发布输出，避免调用方自行拼接路径。"""

    manifest: Path
    evidence: Path


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    prepare_parser = subcommands.add_parser("prepare", help="生成待离线签名的发布清单")
    add_candidate_arguments(prepare_parser)

    descriptor_parser = subcommands.add_parser("descriptor", help="生成一个受约束的产物描述")
    descriptor_parser.add_argument("--root", type=Path, required=True)
    descriptor_parser.add_argument("--output", type=Path, required=True)
    descriptor_parser.add_argument("--name", required=True)
    descriptor_parser.add_argument("--kind", choices=sorted(VALID_KINDS), required=True)
    descriptor_parser.add_argument("--platform", required=True)
    descriptor_parser.add_argument("--path", required=True)
    descriptor_parser.add_argument("--url", required=True)
    descriptor_parser.add_argument("--sbom-path", required=True)
    descriptor_parser.add_argument("--sbom-url", required=True)
    descriptor_parser.add_argument("--signature-path", required=True)
    descriptor_parser.add_argument("--signature-url", required=True)
    descriptor_parser.add_argument("--signature-mode", choices=("blob", "oci"), required=True)

    inventory_parser = subcommands.add_parser("inventory", help="聚合多平台产物描述")
    inventory_parser.add_argument("--root", type=Path, required=True)
    inventory_parser.add_argument("--descriptors", type=Path, required=True)
    inventory_parser.add_argument("--output", type=Path, required=True)
    inventory_parser.add_argument("--channel", choices=sorted(VALID_CHANNELS), required=True)
    inventory_parser.add_argument("--sequence", type=int, required=True)
    inventory_parser.add_argument("--version", required=True)
    inventory_parser.add_argument("--published-at-unix-seconds", type=int, required=True)
    inventory_parser.add_argument("--expires-at-unix-seconds", type=int, required=True)
    inventory_parser.add_argument("--rollback-from")
    inventory_parser.add_argument("--tauri-manifest-url", required=True)

    verify_parser = subcommands.add_parser("verify", help="验证候选、离线签名和 Sigstore 证据")
    add_candidate_arguments(verify_parser)
    verify_parser.add_argument("--signed-manifest", type=Path, required=True)
    verify_parser.add_argument("--public-key", type=Path, required=True)
    verify_parser.add_argument("--release-tool", type=Path, required=True)
    verify_parser.add_argument("--installed-version", required=True)
    verify_parser.add_argument("--highest-sequence", type=int, required=True)
    verify_parser.add_argument("--now-unix-seconds", type=int, required=True)
    verify_parser.add_argument("--cosign", default="cosign")
    verify_parser.add_argument("--certificate-identity-regexp", required=True)
    verify_parser.add_argument(
        "--certificate-oidc-issuer",
        default="https://token.actions.githubusercontent.com",
    )
    return parser.parse_args(argv)


def add_candidate_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)


def load_object(path: Path, label: str) -> Mapping[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ReleaseFailure(f"{label}不是有效 JSON：{path}") from error
    if not isinstance(value, dict):
        raise ReleaseFailure(f"{label}必须是 JSON 对象：{path}")
    return value


def require_string(source: Mapping[str, object], key: str, label: str) -> str:
    value = source.get(key)
    if not isinstance(value, str) or not value:
        raise ReleaseFailure(f"{label}.{key} 必须是非空字符串。")
    return value


def require_integer(source: Mapping[str, object], key: str, label: str) -> int:
    value = source.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ReleaseFailure(f"{label}.{key} 必须是非负整数。")
    return value


def optional_string(source: Mapping[str, object], key: str, label: str) -> str | None:
    value = source.get(key)
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise ReleaseFailure(f"{label}.{key} 必须是非空字符串或 null。")
    return value


def resolve_local_file(root: Path, value: str, label: str) -> Path:
    relative = Path(value)
    if relative.is_absolute() or ".." in relative.parts:
        raise ReleaseFailure(f"{label} 必须是候选根目录内的相对路径。")
    resolved_root = root.resolve(strict=True)
    try:
        resolved = (resolved_root / relative).resolve(strict=True)
        resolved.relative_to(resolved_root)
    except (OSError, ValueError) as error:
        raise ReleaseFailure(f"{label} 不存在或逃逸候选根目录：{value}") from error
    if not resolved.is_file():
        raise ReleaseFailure(f"{label} 不是普通文件：{value}")
    return resolved


def validate_https_url(value: str, label: str) -> None:
    parsed = urlparse(value)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise ReleaseFailure(f"{label} 必须是无内嵌凭据的 HTTPS 地址。")


def validate_artifact_url(value: str, kind: str) -> None:
    if kind != "oci-image":
        validate_https_url(value, "artifact.url")
        return
    if oci_digest(value) is None:
        raise ReleaseFailure("OCI 产物地址必须使用 oci://...@sha256:<digest>。")


def oci_digest(value: str) -> str | None:
    if not value.startswith("oci://"):
        return None
    marker = "@sha256:"
    if marker not in value:
        return None
    repository, digest = value.rsplit(marker, maxsplit=1)
    if not repository.removeprefix("oci://") or not SHA256_PATTERN.fullmatch(digest):
        return None
    return digest


def validate_sbom(path: Path) -> None:
    document = load_object(path, "SBOM")
    if document.get("bomFormat") != "CycloneDX":
        raise ReleaseFailure(f"SBOM 不是 CycloneDX 文档：{path}")
    spec_version = document.get("specVersion")
    if not isinstance(spec_version, str) or not spec_version:
        raise ReleaseFailure(f"SBOM 缺少 specVersion：{path}")
    components = document.get("components")
    if components is not None and not isinstance(components, list):
        raise ReleaseFailure(f"SBOM components 必须是数组：{path}")


def validate_sigstore_bundle(path: Path) -> None:
    bundle = load_object(path, "Sigstore bundle")
    media_type = bundle.get("mediaType")
    if not isinstance(media_type, str) or not media_type.startswith(SIGSTORE_MEDIA_TYPE_PREFIX):
        raise ReleaseFailure(f"签名证据不是受支持的 Sigstore bundle：{path}")
    if not isinstance(bundle.get("verificationMaterial"), dict):
        raise ReleaseFailure(f"Sigstore bundle 缺少 verificationMaterial：{path}")
    if not isinstance(bundle.get("messageSignature"), dict):
        raise ReleaseFailure(f"Sigstore bundle 缺少 messageSignature：{path}")


def parse_artifacts(root: Path, inventory: Mapping[str, object]) -> tuple[ArtifactSource, ...]:
    raw_artifacts = inventory.get("artifacts")
    if not isinstance(raw_artifacts, list) or not raw_artifacts:
        raise ReleaseFailure("inventory.artifacts 必须是非空数组。")

    artifacts: list[ArtifactSource] = []
    identities: set[tuple[str, str, str]] = set()
    for index, value in enumerate(raw_artifacts):
        label = f"inventory.artifacts[{index}]"
        if not isinstance(value, dict):
            raise ReleaseFailure(f"{label} 必须是对象。")
        artifact = parse_artifact(root, value, label)
        identity = (artifact.kind, artifact.platform, artifact.name)
        if identity in identities:
            raise ReleaseFailure(
                f"发布产物身份重复：{artifact.kind}/{artifact.platform}/{artifact.name}"
            )
        identities.add(identity)
        artifacts.append(artifact)

    missing = REQUIRED_KINDS - {artifact.kind for artifact in artifacts}
    if missing:
        raise ReleaseFailure(f"发布候选缺少必需产物类型：{', '.join(sorted(missing))}")
    missing_images = REQUIRED_OCI_IMAGES - {
        artifact.name for artifact in artifacts if artifact.kind == "oci-image"
    }
    if missing_images:
        raise ReleaseFailure(f"发布候选缺少必需 OCI 镜像：{', '.join(sorted(missing_images))}")
    return tuple(artifacts)


def parse_artifact(
    root: Path,
    value: Mapping[str, object],
    label: str,
) -> ArtifactSource:
    name = require_string(value, "name", label)
    kind = require_string(value, "kind", label)
    platform = require_string(value, "platform", label)
    if not NAME_PATTERN.fullmatch(name):
        raise ReleaseFailure(f"{label}.name 不符合发布命名规则。")
    if kind not in VALID_KINDS:
        raise ReleaseFailure(f"{label}.kind 不受支持：{kind}")
    if not PLATFORM_PATTERN.fullmatch(platform):
        raise ReleaseFailure(f"{label}.platform 不符合发布命名规则。")

    url = require_string(value, "url", label)
    sbom_url = require_string(value, "sbomUrl", label)
    signature_url = require_string(value, "signatureUrl", label)
    signature_mode = require_string(value, "signatureMode", label)
    validate_artifact_url(url, kind)
    validate_https_url(sbom_url, f"{label}.sbomUrl")
    validate_https_url(signature_url, f"{label}.signatureUrl")
    if signature_mode not in {"blob", "oci"}:
        raise ReleaseFailure(f"{label}.signatureMode 必须是 blob 或 oci。")
    if (kind == "oci-image") != (signature_mode == "oci"):
        raise ReleaseFailure(f"{label}.signatureMode 与产物类型不一致。")

    artifact = ArtifactSource(
        name=name,
        kind=kind,
        platform=platform,
        path=resolve_local_file(root, require_string(value, "path", label), f"{label}.path"),
        url=url,
        sbom_path=resolve_local_file(
            root,
            require_string(value, "sbomPath", label),
            f"{label}.sbomPath",
        ),
        sbom_url=sbom_url,
        signature_path=resolve_local_file(
            root,
            require_string(value, "signaturePath", label),
            f"{label}.signaturePath",
        ),
        signature_url=signature_url,
        signature_mode=signature_mode,
    )
    validate_sbom(artifact.sbom_path)
    validate_sigstore_bundle(artifact.signature_path)
    return artifact


def artifact_document(artifact: ArtifactSource, root: Path) -> dict[str, object]:
    return {
        "name": artifact.name,
        "kind": artifact.kind,
        "platform": artifact.platform,
        "path": relative_path(root, artifact.path),
        "url": artifact.url,
        "sbomPath": relative_path(root, artifact.sbom_path),
        "sbomUrl": artifact.sbom_url,
        "signaturePath": relative_path(root, artifact.signature_path),
        "signatureUrl": artifact.signature_url,
        "signatureMode": artifact.signature_mode,
    }


def create_descriptor(args: argparse.Namespace) -> None:
    root = args.root.resolve(strict=True)
    raw = {
        "name": args.name,
        "kind": args.kind,
        "platform": args.platform,
        "path": args.path,
        "url": args.url,
        "sbomPath": args.sbom_path,
        "sbomUrl": args.sbom_url,
        "signaturePath": args.signature_path,
        "signatureUrl": args.signature_url,
        "signatureMode": args.signature_mode,
    }
    artifact = parse_artifact(root, raw, "descriptor")
    write_new_json(args.output, artifact_document(artifact, root))


def create_inventory(args: argparse.Namespace) -> None:
    root = args.root.resolve(strict=True)
    descriptors = args.descriptors.resolve(strict=True)
    try:
        descriptors.relative_to(root)
    except ValueError as error:
        raise ReleaseFailure("描述文件目录必须位于候选根目录内。") from error
    descriptor_paths = sorted(descriptors.rglob("*.artifact.json"))
    if not descriptor_paths:
        raise ReleaseFailure("没有找到任何 *.artifact.json 描述文件。")
    artifacts: list[Mapping[str, object]] = []
    for descriptor_path in descriptor_paths:
        artifacts.append(load_object(descriptor_path, "产物描述"))
    inventory: dict[str, object] = {
        "schemaVersion": SCHEMA_VERSION,
        "channel": args.channel,
        "sequence": args.sequence,
        "version": args.version,
        "publishedAtUnixSeconds": args.published_at_unix_seconds,
        "expiresAtUnixSeconds": args.expires_at_unix_seconds,
        "rollbackFrom": args.rollback_from or None,
        "tauriManifestUrl": args.tauri_manifest_url,
        "artifacts": artifacts,
    }
    parse_artifacts(root, inventory)
    write_new_json(args.output, inventory)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def relative_path(root: Path, path: Path) -> str:
    return path.relative_to(root.resolve(strict=True)).as_posix()


def build_candidate(
    root: Path,
    inventory_path: Path,
) -> tuple[dict[str, object], dict[str, object]]:
    resolved_root = root.resolve(strict=True)
    inventory = load_object(inventory_path, "发布 inventory")
    if require_integer(inventory, "schemaVersion", "inventory") != SCHEMA_VERSION:
        raise ReleaseFailure("不支持的发布 inventory schemaVersion。")
    channel = require_string(inventory, "channel", "inventory")
    if channel not in VALID_CHANNELS:
        raise ReleaseFailure("inventory.channel 必须是 stable 或 testing。")
    sequence = require_integer(inventory, "sequence", "inventory")
    if sequence == 0:
        raise ReleaseFailure("inventory.sequence 必须大于零。")
    version = require_string(inventory, "version", "inventory")
    published_at = require_integer(inventory, "publishedAtUnixSeconds", "inventory")
    expires_at = require_integer(inventory, "expiresAtUnixSeconds", "inventory")
    if expires_at <= published_at or expires_at - published_at > 30 * 24 * 60 * 60:
        raise ReleaseFailure("发布清单有效期必须大于零且不超过 30 天。")
    rollback_from = optional_string(inventory, "rollbackFrom", "inventory")
    tauri_manifest_url = optional_string(inventory, "tauriManifestUrl", "inventory")
    if tauri_manifest_url is not None:
        validate_https_url(tauri_manifest_url, "inventory.tauriManifestUrl")

    artifacts = parse_artifacts(resolved_root, inventory)
    manifest_artifacts: list[dict[str, object]] = []
    evidence_artifacts: list[dict[str, object]] = []
    for artifact in artifacts:
        local_digest = sha256_file(artifact.path)
        digest = oci_digest(artifact.url) if artifact.kind == "oci-image" else local_digest
        if digest is None:
            raise ReleaseFailure(f"OCI 地址摘要无效：{artifact.name}")
        byte_length = artifact.path.stat().st_size
        if byte_length == 0:
            raise ReleaseFailure(f"发布产物不能为空：{artifact.path}")
        manifest_artifacts.append(
            {
                "name": artifact.name,
                "kind": artifact.kind,
                "platform": artifact.platform,
                "url": artifact.url,
                "sha256": digest,
                "byteLength": byte_length,
                "sbomUrl": artifact.sbom_url,
                "signatureUrl": artifact.signature_url,
            }
        )
        evidence_artifacts.append(
            {
                "name": artifact.name,
                "kind": artifact.kind,
                "platform": artifact.platform,
                "path": relative_path(resolved_root, artifact.path),
                "url": artifact.url,
                "sha256": digest,
                "localEvidenceSha256": local_digest,
                "byteLength": byte_length,
                "sbomPath": relative_path(resolved_root, artifact.sbom_path),
                "sbomSha256": sha256_file(artifact.sbom_path),
                "signaturePath": relative_path(resolved_root, artifact.signature_path),
                "signatureSha256": sha256_file(artifact.signature_path),
                "signatureMode": artifact.signature_mode,
            }
        )

    if any(artifact.kind == "desktop" for artifact in artifacts) and tauri_manifest_url is None:
        raise ReleaseFailure("包含桌面产物时必须提供 tauriManifestUrl。")

    manifest: dict[str, object] = {
        "schemaVersion": SCHEMA_VERSION,
        "channel": channel,
        "sequence": sequence,
        "version": version,
        "publishedAtUnixSeconds": published_at,
        "expiresAtUnixSeconds": expires_at,
        "rollbackFrom": rollback_from,
        "tauriManifestUrl": tauri_manifest_url,
        "artifacts": manifest_artifacts,
    }
    inventory_bytes = inventory_path.read_bytes()
    evidence: dict[str, object] = {
        "schemaVersion": SCHEMA_VERSION,
        "channel": channel,
        "sequence": sequence,
        "version": version,
        "inventorySha256": hashlib.sha256(inventory_bytes).hexdigest(),
        "artifacts": evidence_artifacts,
    }
    return manifest, evidence


def write_new_json(path: Path, value: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8", newline="\n") as target:
            json.dump(value, target, ensure_ascii=False, indent=2)
            target.write("\n")
    except FileExistsError as error:
        raise ReleaseFailure(f"拒绝覆盖已有发布文件：{path}") from error


def prepare(root: Path, inventory: Path, paths: CandidatePaths) -> None:
    existing = [str(path) for path in (paths.manifest, paths.evidence) if path.exists()]
    if existing:
        raise ReleaseFailure(f"拒绝覆盖已有发布文件：{', '.join(existing)}")
    manifest, evidence = build_candidate(root, inventory)
    write_new_json(paths.manifest, manifest)
    evidence_with_manifest = dict(evidence)
    evidence_with_manifest["manifestSha256"] = sha256_file(paths.manifest)
    write_new_json(paths.evidence, evidence_with_manifest)


def validate_candidate_files(
    root: Path,
    inventory: Path,
    paths: CandidatePaths,
) -> tuple[Mapping[str, object], Mapping[str, object]]:
    expected_manifest, expected_evidence = build_candidate(root, inventory)
    actual_manifest = load_object(paths.manifest, "待签名发布清单")
    actual_evidence = load_object(paths.evidence, "候选证据")
    if actual_manifest != expected_manifest:
        raise ReleaseFailure("待签名发布清单与当前产物不一致。")
    evidence = dict(expected_evidence)
    evidence["manifestSha256"] = sha256_file(paths.manifest)
    if actual_evidence != evidence:
        raise ReleaseFailure("候选证据与当前产物不一致。")
    return actual_manifest, actual_evidence


def decode_signed_payload(path: Path) -> Mapping[str, object]:
    envelope = load_object(path, "离线签名发布清单")
    payload = require_string(envelope, "payload", "signedManifest")
    try:
        padding = "=" * (-len(payload) % 4)
        decoded = base64.urlsafe_b64decode(payload + padding)
        value = json.loads(decoded)
    except (ValueError, UnicodeError, json.JSONDecodeError) as error:
        raise ReleaseFailure("离线签名清单的 payload 无法解码。") from error
    if not isinstance(value, dict):
        raise ReleaseFailure("离线签名清单的 payload 必须是对象。")
    return value


def run_checked(command: Sequence[str], label: str) -> None:
    try:
        completed = subprocess.run(
            command,
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
    except OSError as error:
        raise ReleaseFailure(f"{label}无法启动：{command[0]}") from error
    if completed.returncode != 0:
        output = completed.stdout.strip()
        raise ReleaseFailure(f"{label}失败（退出码 {completed.returncode}）：\n{output}")


def verify_offline_signature(
    release_tool: Path,
    public_key: Path,
    signed_manifest: Path,
    manifest: Mapping[str, object],
    installed_version: str,
    highest_sequence: int,
    now_unix_seconds: int,
) -> None:
    if highest_sequence < 0 or now_unix_seconds < 0:
        raise ReleaseFailure("可信序号和验证时间不能为负数。")
    channel = require_string(manifest, "channel", "manifest")
    run_checked(
        (
            str(release_tool),
            "verify",
            "--public-key",
            str(public_key),
            "--manifest",
            str(signed_manifest),
            "--channel",
            channel,
            "--installed-version",
            installed_version,
            "--highest-sequence",
            str(highest_sequence),
            "--now-unix-seconds",
            str(now_unix_seconds),
        ),
        "离线发布签名验证",
    )
    if decode_signed_payload(signed_manifest) != manifest:
        raise ReleaseFailure("离线签名 payload 与候选清单不一致。")


def verify_sigstore_evidence(
    root: Path,
    inventory: Mapping[str, object],
    cosign: str,
    certificate_identity_regexp: str,
    certificate_oidc_issuer: str,
) -> None:
    artifacts = parse_artifacts(root, inventory)
    for artifact in artifacts:
        base = (
            "--bundle",
            str(artifact.signature_path),
            "--certificate-identity-regexp",
            certificate_identity_regexp,
            "--certificate-oidc-issuer",
            certificate_oidc_issuer,
        )
        if artifact.signature_mode == "blob":
            command = (cosign, "verify-blob", *base, str(artifact.path))
        else:
            command = (cosign, "verify", *base, artifact.url.removeprefix("oci://"))
        run_checked(command, f"Sigstore 证据验证 {artifact.kind}/{artifact.name}")


def verify(args: argparse.Namespace) -> None:
    paths = CandidatePaths(args.manifest, args.evidence)
    manifest, _ = validate_candidate_files(args.root, args.inventory, paths)
    verify_offline_signature(
        args.release_tool,
        args.public_key,
        args.signed_manifest,
        manifest,
        args.installed_version,
        args.highest_sequence,
        args.now_unix_seconds,
    )
    inventory = load_object(args.inventory, "发布 inventory")
    verify_sigstore_evidence(
        args.root,
        inventory,
        args.cosign,
        args.certificate_identity_regexp,
        args.certificate_oidc_issuer,
    )


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        if args.command == "descriptor":
            create_descriptor(args)
            print(f"已生成产物描述：{args.output}")
        elif args.command == "inventory":
            create_inventory(args)
            print(f"已聚合发布 inventory：{args.output}")
        elif args.command == "prepare":
            paths = CandidatePaths(args.manifest, args.evidence)
            prepare(args.root, args.inventory, paths)
            print(f"已生成待签名清单：{paths.manifest}")
            print(f"已生成候选证据：{paths.evidence}")
        else:
            paths = CandidatePaths(args.manifest, args.evidence)
            verify(args)
            print("发布候选、离线签名和 Sigstore 证据均验证通过。")
        return 0
    except ReleaseFailure as error:
        print(f"发布候选失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
