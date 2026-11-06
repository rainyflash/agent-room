"""注册页（Keycloak 主题）不让提交发信服务不收的邮箱地址。"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTER_PAGE = ROOT / "infra/identity/themes/agent-room/login/register.ftl"


class RegisterPageTests(unittest.TestCase):
    def test_email_needs_a_domain_suffix(self) -> None:
        field = re.search(r'<input id="email".*?/>', REGISTER_PAGE.read_text(encoding="utf-8"), re.S)
        assert field is not None
        pattern = re.search(r'pattern="([^"]+)"', field.group(0))
        assert pattern is not None
        # 2026-11-02 生产上发信服务拒收的就是 @ 后面只有数字的地址（agent-room-ops#62）。
        for refused in ("name@1234", "name@example", "name@example.c", "name@example."):
            self.assertIsNone(re.fullmatch(pattern.group(1), refused), refused)
        for accepted in ("name@example.com", "a.b+c@sub.example.co", "developer@agent-room.test"):
            self.assertIsNotNone(re.fullmatch(pattern.group(1), accepted), accepted)
        self.assertIn('msg("invalidEmailMessage")', field.group(0))


if __name__ == "__main__":
    unittest.main()
