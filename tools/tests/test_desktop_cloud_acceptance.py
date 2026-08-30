from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.desktop_cloud_acceptance import (
    VerticalFailure,
    desktop_acceptance_environment,
    playwright_acceptance_command,
    playwright_acceptance_environment,
    tauri_acceptance_build_command,
    validate_evidence,
)


class DesktopCloudAcceptanceTests(unittest.TestCase):
    def test_桌面环境隔离用户配置并强制_bridge_离线(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.dict(
                "os.environ",
                {
                    "AGENT_ROOM_AGENT_ID": "不得泄漏",
                    "AGENT_ROOM_CONTROL_PLANE_URL": "https://production.invalid",
                    "KEEP_ME": "保留",
                    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "旧参数",
                },
                clear=True,
            ):
                environment = desktop_acceptance_environment(
                    root,
                    cdp_port=9_223,
                    secure_storage_service="dev.agent-room.acceptance.test",
                )

        self.assertEqual(environment["KEEP_ME"], "保留")
        self.assertNotIn("AGENT_ROOM_AGENT_ID", environment)
        self.assertEqual(
            environment["AGENT_ROOM_CONTROL_PLANE_URL"],
            "http://127.0.0.1:1",
        )
        self.assertEqual(
            environment["AGENT_ROOM_BRIDGE_DATA_DIR"],
            str(root / "bridge-data"),
        )
        self.assertEqual(
            environment["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"],
            "--remote-debugging-port=9223 --ignore-certificate-errors",
        )

    def test_tauri_构建同时加载_sidecar_与验收安全配置(self) -> None:
        with patch(
            "tools.desktop_cloud_acceptance.executable",
            return_value="corepack",
        ):
            command = tauri_acceptance_build_command()

        self.assertIn("--no-bundle", command)
        self.assertEqual(command.count("--config"), 2)
        self.assertIn("src-tauri/tauri.sidecar.conf.json", command)
        self.assertIn("src-tauri/tauri.acceptance.conf.json", command)

    def test_playwright_命令只运行真实桌面闭环场景(self) -> None:
        with patch(
            "tools.desktop_cloud_acceptance.executable",
            return_value="node",
        ):
            command = playwright_acceptance_command()

        self.assertEqual(command[-1], "desktop-cloud-closure.e2e.ts")
        self.assertNotIn("password", " ".join(command).lower())

    def test_playwright_环境不继承其他_agent_room_凭据(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            with patch.dict(
                "os.environ",
                {
                    "AGENT_ROOM_PRODUCTION_SECRET": "不得泄漏",
                    "KEEP_ME": "保留",
                },
                clear=True,
            ):
                environment = playwright_acceptance_environment(
                    cdp_port=9_223,
                    password="隔离夹具密码",
                    result_path=result,
                )

        self.assertNotIn("AGENT_ROOM_PRODUCTION_SECRET", environment)
        self.assertEqual(environment["KEEP_ME"], "保留")
        self.assertEqual(
            environment["AGENT_ROOM_DESKTOP_ACCEPTANCE_PASSWORD"],
            "隔离夹具密码",
        )

    def test_验收结果必须证明真实_tauri_与_bridge_离线闭环(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text(
                json.dumps(
                    {
                        "bridgePhase": "halted",
                        "controlPlaneStatus": "online",
                        "desktopOrigin": "http://tauri.localhost",
                        "lobbyEntered": "true",
                        "lobbyName": "Vertical Codex Lobby",
                        "matrixStatus": "online",
                        "processKind": "tauri_webview2",
                        "tauriRuntimeDetected": "true",
                        "workspaceVisible": "true",
                    }
                ),
                encoding="utf-8",
            )

            evidence = validate_evidence(result)

        self.assertEqual(evidence["bridgePhase"], "halted")

    def test_拒绝把普通浏览器伪装成桌面验收(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text(
                json.dumps(
                    {
                        "bridgePhase": "halted",
                        "controlPlaneStatus": "online",
                        "desktopOrigin": "https://app.agent-room.localhost:18443",
                        "lobbyEntered": "true",
                        "lobbyName": "Vertical Codex Lobby",
                        "matrixStatus": "online",
                        "processKind": "tauri_webview2",
                        "tauriRuntimeDetected": "true",
                        "workspaceVisible": "true",
                    }
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(VerticalFailure, "真实 Tauri"):
                validate_evidence(result)


if __name__ == "__main__":
    unittest.main()
