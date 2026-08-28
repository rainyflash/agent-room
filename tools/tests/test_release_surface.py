from __future__ import annotations

import unittest
from unittest.mock import patch

from tools.release_surface import (
    GhCliReleaseGateway,
    ReleaseSurfaceFailure,
    asset_label,
    build_plan,
    installer_name,
)


class ReleaseSurfaceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.repository = "rainyflash/agent-room"
        self.version = "0.1.0-alpha.1"
        self.tag = f"v{self.version}"
        self.installer = installer_name(self.version)

    def test_普通用户安装器是唯一下载入口(self) -> None:
        plan = build_plan(
            self.repository,
            self.tag,
            self.version,
            {"id": 42},
            [
                {"id": 1, "name": self.installer, "label": None},
                {
                    "id": 2,
                    "name": "agent-room-bridge-v0.1.0-alpha.1-windows-x86_64.exe",
                    "label": None,
                },
                {
                    "id": 3,
                    "name": "agent-room-installer-v0.1.0-alpha.1-windows-x86_64.exe.cdx.json",
                    "label": None,
                },
            ],
        )

        labels = {update.name: update.label for update in plan.asset_updates}
        self.assertTrue(labels[self.installer].startswith("DOWNLOAD / 下载"))
        self.assertIn("不要单独运行", labels[next(name for name in labels if "bridge" in name)])
        self.assertIn("无需下载", labels[next(name for name in labels if name.endswith("cdx.json"))])
        self.assertIn(self.installer, plan.body)

    def test_已经正确标记的资产不会重复更新(self) -> None:
        label = asset_label(self.installer, self.installer)
        plan = build_plan(
            self.repository,
            self.tag,
            self.version,
            {"id": 42},
            [{"id": 1, "name": self.installer, "label": label}],
        )

        self.assertEqual(plan.asset_updates, ())

    def test_缺少安装器会响亮失败(self) -> None:
        with self.assertRaisesRegex(ReleaseSurfaceFailure, "安装器"):
            build_plan(
                self.repository,
                self.tag,
                self.version,
                {"id": 42},
                [{"id": 2, "name": "agent-room-bridge.exe", "label": None}],
            )

    def test_版本和标签必须精确绑定(self) -> None:
        with self.assertRaisesRegex(ReleaseSurfaceFailure, "tag"):
            build_plan(
                self.repository,
                "v0.2.0",
                self.version,
                {"id": 42},
                [{"id": 1, "name": self.installer, "label": None}],
            )

    @patch.object(
        GhCliReleaseGateway,
        "_json",
        return_value=[
            [
                {"id": 41, "tag_name": "v0.0.9"},
                {"id": 42, "tag_name": "v0.1.0-alpha.1", "draft": True},
            ]
        ],
    )
    def test_github_release分页包含草稿并精确匹配标签(self, request) -> None:
        gateway = GhCliReleaseGateway()

        release = gateway.get_release(self.repository, self.tag)

        self.assertEqual(release["id"], 42)
        request.assert_called_once_with(
            (
                "--paginate",
                "--slurp",
                f"repos/{self.repository}/releases?per_page=100",
            )
        )

    @patch.object(
        GhCliReleaseGateway,
        "_json",
        return_value=[[{"id": 41, "tag_name": "v0.0.9"}]],
    )
    def test_github_release缺少目标标签会响亮失败(self, _request) -> None:
        gateway = GhCliReleaseGateway()

        with self.assertRaisesRegex(ReleaseSurfaceFailure, "实际为 0 个"):
            gateway.get_release(self.repository, self.tag)


if __name__ == "__main__":
    unittest.main()
