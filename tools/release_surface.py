#!/usr/bin/env python3
"""整理 GitHub Release 面向普通用户的下载界面。"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import re
import subprocess
import sys
from typing import Final, Mapping, Protocol, Sequence


API_VERSION: Final = "2026-03-10"
REPOSITORY_PATTERN: Final = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
SEMVER_PATTERN: Final = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:[-+][0-9A-Za-z.-]+)?$"
)


class ReleaseSurfaceFailure(RuntimeError):
    """表示 Release 下载界面无法被安全整理。"""


@dataclass(frozen=True)
class ReleaseAsset:
    asset_id: int
    name: str
    label: str | None


@dataclass(frozen=True)
class AssetLabelUpdate:
    asset_id: int
    name: str
    label: str


@dataclass(frozen=True)
class ReleaseSurfacePlan:
    release_id: int
    title: str
    body: str
    asset_updates: tuple[AssetLabelUpdate, ...]


class ReleaseGateway(Protocol):
    def get_release(self, repository: str, tag: str) -> Mapping[str, object]: ...

    def list_assets(self, repository: str, release_id: int) -> Sequence[Mapping[str, object]]: ...

    def update_release(self, repository: str, plan: ReleaseSurfacePlan) -> None: ...

    def update_asset_label(
        self, repository: str, update: AssetLabelUpdate
    ) -> None: ...

    def publish_release(
        self, repository: str, release_id: int, prerelease: bool
    ) -> None: ...


class GhCliReleaseGateway:
    """通过已认证的 GitHub CLI 适配 Release REST API。"""

    def get_release(self, repository: str, tag: str) -> Mapping[str, object]:
        value = self._json(
            (
                "--paginate",
                "--slurp",
                f"repos/{repository}/releases?per_page=100",
            )
        )
        if not isinstance(value, list):
            raise ReleaseSurfaceFailure("GitHub Release 分页响应必须是数组。")
        matches: list[Mapping[str, object]] = []
        for page in value:
            if not isinstance(page, list):
                raise ReleaseSurfaceFailure("GitHub Release 分页响应无效。")
            for item in page:
                release = require_mapping(item, "release")
                if release.get("tag_name") == tag:
                    matches.append(release)
        if len(matches) != 1:
            raise ReleaseSurfaceFailure(
                f"必须且只能找到一个标签为 {tag} 的 Release，实际为 {len(matches)} 个。"
            )
        return matches[0]

    def list_assets(self, repository: str, release_id: int) -> Sequence[Mapping[str, object]]:
        value = self._json(
            (
                "--paginate",
                "--slurp",
                f"repos/{repository}/releases/{release_id}/assets?per_page=100",
            )
        )
        if not isinstance(value, list):
            raise ReleaseSurfaceFailure("GitHub Release 资产响应必须是数组。")
        assets: list[Mapping[str, object]] = []
        for page in value:
            if not isinstance(page, list):
                raise ReleaseSurfaceFailure("GitHub Release 资产分页响应无效。")
            assets.extend(require_mapping(item, "asset") for item in page)
        return tuple(assets)

    def update_release(self, repository: str, plan: ReleaseSurfacePlan) -> None:
        self._run(
            (
                "--method",
                "PATCH",
                f"repos/{repository}/releases/{plan.release_id}",
                "-f",
                f"name={plan.title}",
                "-f",
                f"body={plan.body}",
            )
        )

    def update_asset_label(self, repository: str, update: AssetLabelUpdate) -> None:
        self._run(
            (
                "--method",
                "PATCH",
                f"repos/{repository}/releases/assets/{update.asset_id}",
                "-f",
                f"label={update.label}",
            )
        )

    def publish_release(
        self, repository: str, release_id: int, prerelease: bool
    ) -> None:
        arguments = [
            "--method",
            "PATCH",
            f"repos/{repository}/releases/{release_id}",
            "-F",
            "draft=false",
            "-F",
            f"prerelease={'true' if prerelease else 'false'}",
        ]
        if not prerelease:
            arguments.extend(("-f", "make_latest=true"))
        self._run(tuple(arguments))

    @staticmethod
    def _run(arguments: Sequence[str]) -> str:
        command = (
            "gh",
            "api",
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            f"X-GitHub-Api-Version: {API_VERSION}",
            *arguments,
        )
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            encoding="utf-8",
            errors="replace",
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip() or completed.stdout.strip()
            raise ReleaseSurfaceFailure(f"GitHub API 调用失败：{detail}")
        return completed.stdout

    def _json(self, arguments: Sequence[str]) -> object:
        payload = self._run(arguments)
        try:
            return json.loads(payload)
        except json.JSONDecodeError as error:
            raise ReleaseSurfaceFailure("GitHub API 返回了无效 JSON。") from error


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("apply", "plan", "publish"))
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--channel", choices=("stable", "testing"))
    return parser.parse_args(argv)


def require_mapping(value: object, label: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise ReleaseSurfaceFailure(f"{label} 必须是 JSON 对象。")
    return value


def require_text(source: Mapping[str, object], key: str, label: str) -> str:
    value = source.get(key)
    if not isinstance(value, str) or not value:
        raise ReleaseSurfaceFailure(f"{label}.{key} 必须是非空字符串。")
    return value


def require_positive_integer(source: Mapping[str, object], key: str, label: str) -> int:
    value = source.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise ReleaseSurfaceFailure(f"{label}.{key} 必须是正整数。")
    return value


def validate_inputs(repository: str, tag: str, version: str) -> None:
    if not REPOSITORY_PATTERN.fullmatch(repository):
        raise ReleaseSurfaceFailure("repository 必须是 OWNER/REPO。")
    if not SEMVER_PATTERN.fullmatch(version):
        raise ReleaseSurfaceFailure("version 必须是 SemVer。")
    if tag != f"v{version}":
        raise ReleaseSurfaceFailure("tag 必须严格等于 v 加 version。")


def installer_name(version: str) -> str:
    return f"agent-room-installer-v{version}-windows-x86_64.exe"


def release_download_url(repository: str, tag: str, asset_name: str) -> str:
    return f"https://github.com/{repository}/releases/download/{tag}/{asset_name}"


def release_notes(repository: str, tag: str, version: str) -> str:
    download_url = release_download_url(repository, tag, installer_name(version))
    return f"""## Download for Windows / Windows 下载

[**Download Agent Room for Windows x64 / 下载 Agent Room Windows 安装程序**]({download_url})

普通用户只需要下载并运行上面的安装程序。不要单独运行 Bridge、MCP、desktop update payload 或验证文件。

Normal users only need the installer linked above. Do not run the Bridge, MCP, desktop update payload, or verification files separately.

> Alpha software: Windows may show a SmartScreen warning because the installer is not commercially code-signed yet. Choose **More info → Run anyway** only after confirming the download came from this repository.

## What is in the remaining asset list?

The remaining files are automatic-update payloads, host integration packages, SBOMs, signatures, and release evidence for maintainers and advanced integrators. Their labels explicitly say when they must not be run manually.

**Full changelog:** https://github.com/{repository}/commits/{tag}
"""


def asset_label(name: str, expected_installer: str) -> str:
    if name == expected_installer:
        return "DOWNLOAD / 下载：Windows 安装程序（运行这个文件）"
    if name.endswith(".cdx.json"):
        return "VERIFY / 验证文件：CycloneDX SBOM（无需下载）"
    if name.endswith(".sigstore.json"):
        return "VERIFY / 验证文件：Sigstore 证明（无需下载）"
    if name.endswith(".artifact.json"):
        return "VERIFY / 验证文件：产物元数据（无需下载）"
    if name.endswith(".sig"):
        return "VERIFY / 验证文件：自动更新签名（无需下载）"
    if name.startswith("agent-room-bridge-") and name.endswith(".exe"):
        return "INTERNAL / 内部组件：Bridge（不要单独运行）"
    if name.startswith("agent-room-mcp-") and name.endswith(".exe"):
        return "INTERNAL / 内部组件：MCP（不要单独运行）"
    if name.startswith("agent-room-desktop-") and name.endswith(".exe"):
        return "INTERNAL / 内部组件：自动更新载荷（不要手动运行）"
    if name.startswith("agent-room-codex-plugin-"):
        return "ADVANCED / 高级集成：Codex 适配包（安装器会自动配置）"
    if name.startswith("agent-room-update-"):
        return "INTERNAL / 内部组件：自动更新清单（无需下载）"
    if name.endswith(".oci-manifest.json"):
        return "SERVER / 服务端：OCI 镜像清单（普通用户无需下载）"
    if "release" in name or "evidence" in name or "promotion" in name:
        return "MAINTAINER / 维护者：签名发布与晋级证据（无需下载）"
    return "DEVELOPER / 开发者产物：普通用户无需下载"


def parse_assets(values: Sequence[Mapping[str, object]]) -> tuple[ReleaseAsset, ...]:
    assets: list[ReleaseAsset] = []
    for index, value in enumerate(values):
        label = value.get("label")
        if label is not None and not isinstance(label, str):
            raise ReleaseSurfaceFailure(f"asset[{index}].label 必须是字符串或 null。")
        assets.append(
            ReleaseAsset(
                asset_id=require_positive_integer(value, "id", f"asset[{index}]"),
                name=require_text(value, "name", f"asset[{index}]"),
                label=label,
            )
        )
    return tuple(assets)


def build_plan(
    repository: str,
    tag: str,
    version: str,
    release: Mapping[str, object],
    assets: Sequence[Mapping[str, object]],
) -> ReleaseSurfacePlan:
    validate_inputs(repository, tag, version)
    release_id = require_positive_integer(release, "id", "release")
    parsed_assets = parse_assets(assets)
    expected_installer = installer_name(version)
    matching_installers = [asset for asset in parsed_assets if asset.name == expected_installer]
    if len(matching_installers) != 1:
        raise ReleaseSurfaceFailure(
            f"Release 必须且只能包含一个普通用户安装器：{expected_installer}。"
        )
    updates = tuple(
        AssetLabelUpdate(
            asset_id=asset.asset_id,
            name=asset.name,
            label=asset_label(asset.name, expected_installer),
        )
        for asset in parsed_assets
        if asset.label != asset_label(asset.name, expected_installer)
    )
    return ReleaseSurfacePlan(
        release_id=release_id,
        title=f"Agent Room {tag} — Windows Alpha",
        body=release_notes(repository, tag, version),
        asset_updates=updates,
    )


def plan_document(plan: ReleaseSurfacePlan) -> Mapping[str, object]:
    return {
        "releaseId": plan.release_id,
        "title": plan.title,
        "body": plan.body,
        "assetUpdates": [
            {"id": update.asset_id, "name": update.name, "label": update.label}
            for update in plan.asset_updates
        ],
    }


def apply_plan(
    gateway: ReleaseGateway, repository: str, plan: ReleaseSurfacePlan
) -> None:
    gateway.update_release(repository, plan)
    for update in plan.asset_updates:
        gateway.update_asset_label(repository, update)


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        gateway = GhCliReleaseGateway()
        release = gateway.get_release(args.repository, args.tag)
        release_id = require_positive_integer(release, "id", "release")
        assets = gateway.list_assets(args.repository, release_id)
        plan = build_plan(args.repository, args.tag, args.version, release, assets)
        if args.command == "apply":
            apply_plan(gateway, args.repository, plan)
        elif args.command == "publish":
            if args.channel is None:
                raise ReleaseSurfaceFailure("publish 命令必须显式提供 channel。")
            gateway.publish_release(
                args.repository,
                plan.release_id,
                prerelease=args.channel != "stable",
            )
        else:
            json.dump(plan_document(plan), sys.stdout, ensure_ascii=False, indent=2)
            sys.stdout.write("\n")
        return 0
    except ReleaseSurfaceFailure as error:
        print(f"Release 下载界面整理失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
