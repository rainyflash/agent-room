"""登录页（Keycloak 主题）、桌面端回跳页和网页端用同一套颜色令牌和同一个标志。"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UI_SYSTEM_STYLES = ROOT / "packages/ui-system/src/styles.css"
LOGIN_THEME = ROOT / "infra/identity/themes/agent-room/login"
WEB_MARK = ROOT / "apps/web/public/agent-room-mark.svg"


def light_tokens(css: str) -> dict[str, str]:
    """第一个 :root 块里的 --ar-* 令牌，也就是亮色的那一套。"""
    block = re.search(r":root\s*\{(.*?)\}", css, re.S)
    assert block is not None
    return {name: " ".join(value.split()).lower()
            for name, value in re.findall(r"(--ar-[\w-]+)\s*:\s*([^;]+);", block.group(1))}


def text(path: Path) -> str:
    return path.read_text(encoding="utf-8").replace("\r\n", "\n")


class BrandConsistencyTests(unittest.TestCase):
    def test_login_theme_colors_and_outlines_match_the_web_app(self):
        web = light_tokens(text(UI_SYSTEM_STYLES))
        theme = light_tokens(text(LOGIN_THEME / "resources/css/agent-room.css"))
        shared = sorted(set(web) & set(theme))
        # 颜色、描边、圆角、阴影都在里面；少于这些说明有人改了名字，对比就失效了。
        for name in ("--ar-coral", "--ar-outline", "--ar-outline-width", "--ar-panel-radius", "--ar-press"):
            self.assertIn(name, shared)
        drift = {name: (theme[name], web[name]) for name in shared if theme[name] != web[name]}
        self.assertEqual(drift, {})

    def test_login_theme_and_desktop_use_the_web_mark(self):
        mark = text(WEB_MARK)
        self.assertEqual(text(LOGIN_THEME / "resources/img/agent-room-mark.svg"), mark)
        self.assertEqual(text(ROOT / "apps/desktop/src-tauri/icons/agent-room-mark.svg"), mark)
        self.assertIn("img/agent-room-mark.svg", text(LOGIN_THEME / "scene.ftl"))
        self.assertIn("img/agent-room-mark.svg", text(LOGIN_THEME / "template.ftl"))


if __name__ == "__main__":
    unittest.main()
