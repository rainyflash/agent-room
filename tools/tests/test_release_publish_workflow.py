from __future__ import annotations

from pathlib import Path
import re
import unittest
from urllib.parse import urlsplit

from tools.prodops.render import BROWSER_CONTROL_PLANE_PATH_PREFIX


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release-publish.yml"
CANDIDATE_WORKFLOW = ROOT / ".github" / "workflows" / "release-candidate.yml"


class ReleasePublishWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_草稿发布不依赖尚未创建的_git_tag(self) -> None:
        self.assertNotIn('git rev-list -n 1 "$TAG"', self.workflow)
        self.assertIn("candidate/release-metadata.json", self.workflow)

    def test_候选元数据绑定标签版本和真实_git_commit(self) -> None:
        self.assertIn(".tag | select(type == \"string\")", self.workflow)
        self.assertIn(".version | select(type == \"string\")", self.workflow)
        self.assertIn('test("^[0-9a-f]{40}$")', self.workflow)
        self.assertIn('git rev-parse --verify "$REVISION^{commit}"', self.workflow)

    def test_签名和验证统一锁定已修补的_cosign(self) -> None:
        candidate = CANDIDATE_WORKFLOW.read_text(encoding="utf-8")

        self.assertEqual(self.workflow.count("cosign-release: v3.1.3"), 1)
        self.assertEqual(candidate.count("cosign-release: v3.1.3"), 3)
        self.assertEqual(
            self.workflow.count("sigstore/cosign-installer"),
            self.workflow.count("cosign-release: v3.1.3"),
        )
        self.assertEqual(
            candidate.count("sigstore/cosign-installer"),
            candidate.count("cosign-release: v3.1.3"),
        )

    def test_发布前后都整理普通用户下载入口(self) -> None:
        command = "python tools/release_surface.py apply"
        publish = self.workflow.index("python tools/release_surface.py publish")
        first_surface = self.workflow.index(command)
        final_surface = self.workflow.rindex(command)

        self.assertLess(first_surface, publish)
        self.assertGreater(final_surface, publish)

    def test_alpha候选默认只构建客户端(self) -> None:
        candidate = CANDIDATE_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("default: client", candidate)
        self.assertEqual(candidate.count("if: ${{ inputs.profile == 'full' }}"), 2)
        self.assertIn('--profile "${{ inputs.profile }}"', candidate)

    def test_浏览器候选使用同源代理并保留桌面独立会话域(self) -> None:
        candidate = CANDIDATE_WORKFLOW.read_text(encoding="utf-8")
        browser_urls = re.findall(
            r"^\s+VITE_AGENT_ROOM_CONTROL_PLANE_URL=(\S+)\s*$", candidate, re.MULTILINE
        )
        desktop_urls = re.findall(
            r"^\s+VITE_AGENT_ROOM_CONTROL_PLANE_URL: (\S+)\s*$", candidate, re.MULTILINE
        )

        self.assertEqual(len(browser_urls), 1)
        self.assertEqual(len(desktop_urls), 1)
        browser = urlsplit(browser_urls[0])
        desktop = urlsplit(desktop_urls[0])
        # 浏览器登录和回调必须在同一主机，才能携带 __Host 登录 Cookie。
        self.assertEqual(browser.scheme, "https")
        self.assertEqual(browser.netloc, "app.room.the-zeroth.com")
        self.assertEqual(browser.path, BROWSER_CONTROL_PLANE_PATH_PREFIX)
        self.assertEqual((browser.query, browser.fragment), ("", ""))
        # 桌面兑换授权后使用 API 域 Cookie，不能跟着浏览器构建配置一起改域。
        self.assertEqual(desktop_urls, ["https://api.room.the-zeroth.com"])
        self.assertNotEqual(browser.netloc, desktop.netloc)

    def test客户端候选先做廉价静态契约再做真实协议门禁(self) -> None:
        candidate = CANDIDATE_WORKFLOW.read_text(encoding="utf-8")

        contract_gate = candidate.index("在占用原生构建资源前校验 MCP 插件静态契约")
        fixed_dependencies = candidate.index("安装固定依赖")
        desktop_build = candidate.index("构建带强制签名更新的桌面端")
        runtime_gate = candidate.index("使用已构建二进制执行 MCP 真实协议门禁")

        self.assertLess(contract_gate, fixed_dependencies)
        self.assertLess(contract_gate, desktop_build)
        self.assertLess(desktop_build, runtime_gate)
        self.assertIn("python tools/plugin.py validate", candidate)
        self.assertNotRegex(candidate, r"cargo build[^\n]*agent-room-mcp")
        self.assertEqual(candidate.count("python tools/plugin.py stage"), 1)
        self.assertNotIn("cache-to: type=gha,mode=max", candidate)
        self.assertIn("cache-to: type=gha,mode=min", candidate)


if __name__ == "__main__":
    unittest.main()
