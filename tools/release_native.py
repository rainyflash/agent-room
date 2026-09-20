"""Verify the candidate updater payloads and native installer acceptance receipt."""
from __future__ import annotations

import argparse
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import release

INSTALLER_CHECKS = frozenset({
    "silentInstall", "desktopPresent", "bridgePresent", "mcpPresent", "desktopVersion",
    "desktopLaunch", "managedBridgeLaunch", "mcpLaunch", "runningUpgrade",
    "upgradeStoppedDesktop", "upgradeStoppedBridge", "upgradeStoppedMcp",
    "postUpgradeDesktopLaunch", "postUpgradeBridgeLaunch", "postUpgradeMcpLaunch",
    "silentUninstall", "uninstallStoppedRuntime", "installFilesRemoved",
})
MACOS_BUNDLE_CHECKS = frozenset({
    "imageMounted", "appInstalled", "desktopPresent", "bridgePresent", "mcpPresent",
    "cliPresent", "desktopVersion", "cliVersion", "desktopLaunch", "managedBridgeLaunch",
    "mcpLaunch", "desktopExitedWithBridge", "replaceInstall", "postReplaceDesktopLaunch",
    "postReplaceBridgeLaunch", "postReplaceDesktopExitedWithBridge", "applicationRemoved",
})
# 每个有安装器的平台都要带一份真实安装验收回执；没有对应回执的平台不能进发行。
ACCEPTANCE_RECEIPTS = {
    "windows-x86_64": ("windows-installer-acceptance.json", INSTALLER_CHECKS),
    "darwin-aarch64": ("macos-bundle-acceptance.json", MACOS_BUNDLE_CHECKS),
}


def verify(root: Path, tool: Path, configuration: Path) -> None:
    inventory = release.load_object(root / "release-inventory.json", "inventory")
    metadata = release.load_object(root / "release-metadata.json", "metadata")
    artifacts = release.parse_artifacts(root, inventory)
    updates = [artifact for artifact in artifacts if artifact.kind == "update-manifest"]
    if len(updates) != 1:
        raise release.ReleaseFailure("候选必须包含唯一的更新清单。")
    updater = release.load_object(updates[0].path, "updater")
    platforms = updater.get("platforms")
    if not isinstance(platforms, dict) or updater.get("version") != metadata.get("version"):
        raise release.ReleaseFailure("更新清单版本或平台无效。")
    config = release.load_object(configuration, "updater trust configuration")
    plugins = config.get("plugins")
    trust = plugins.get("updater") if isinstance(plugins, dict) else None
    key = trust.get("pubkey") if isinstance(trust, dict) else None
    if not isinstance(key, str) or not key:
        raise release.ReleaseFailure("缺少随受审提交固定的 Tauri 信任公钥。")
    desktops = [artifact for artifact in artifacts if artifact.kind == "desktop"]
    if set(platforms) != {artifact.platform for artifact in desktops}:
        raise release.ReleaseFailure("更新平台与实际桌面产物不一致。")
    for artifact in desktops:
        entry = platforms[artifact.platform]
        if not isinstance(entry, dict) or entry.get("url") != artifact.url or not isinstance(entry.get("signature"), str):
            raise release.ReleaseFailure("更新载荷地址或签名缺失。")
        # The native verifier accepts a signature file, not the base64 document
        # embedded in the updater manifest. Keep it outside the verified assets.
        with tempfile.TemporaryDirectory(prefix="agent-room-update-signature-") as directory:
            signature = Path(directory) / "payload.sig"
            signature.write_text(entry["signature"], encoding="utf-8")
            release.run_checked((str(tool), "verify-tauri", "--public-key", key, "--payload", str(artifact.path), "--signature", str(signature)), "Tauri payload signature")
    installers = [artifact for artifact in artifacts if artifact.kind == "installer"]
    if not any(installer.platform == "windows-x86_64" for installer in installers):
        raise release.ReleaseFailure("缺少 Windows 安装及运行升级验收。")
    for installer in installers:
        receipt_name = ACCEPTANCE_RECEIPTS.get(installer.platform)
        if receipt_name is None:
            raise release.ReleaseFailure(f"没有为 {installer.platform} 定义安装验收回执。")
        filename, required_checks = receipt_name
        receipt = release.load_object(root / filename, "installer acceptance")
        installed = receipt.get("installer")
        checks = receipt.get("checks")
        if not isinstance(installed, dict) or not isinstance(checks, dict):
            raise release.ReleaseFailure(f"缺少 {installer.platform} 的安装验收。")
        if receipt.get("schemaVersion") != 1 or receipt.get("result") != "passed" or receipt.get("version") != metadata.get("version") or receipt.get("platform") != installer.platform or installed.get("sha256") != release.sha256_file(installer.path) or any(checks.get(key) is not True for key in required_checks):
            raise release.ReleaseFailure("安装器验收不完整或不属于当前安装器。")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--release-tool", type=Path, required=True)
    args = parser.parse_args()
    try:
        verify(args.root, args.release_tool, ROOT / "apps/desktop/src-tauri/tauri.release.conf.json")
    except (release.ReleaseFailure, OSError, ValueError) as error:
        parser.exit(1, f"原生产物验收未通过：{error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
