from __future__ import annotations

import subprocess
import unittest
from unittest.mock import MagicMock, patch

from tools.web import run_live_session_test


class WebAcceptanceTests(unittest.TestCase):
    @patch("tools.web.subprocess.run")
    @patch("tools.web.shutil.which", return_value="/usr/bin/corepack")
    def test_真实会话通过带协议构建前置条件的脚本启动(
        self,
        _which: MagicMock,
        run: MagicMock,
    ) -> None:
        completed = subprocess.CompletedProcess([], 0)
        run.return_value = completed

        result = run_live_session_test({"SEED_ADMIN_PASSWORD": "test-password"})

        self.assertEqual(result, 0)
        command = run.call_args.args[0]
        self.assertEqual(
            command,
            [
                "/usr/bin/corepack",
                "pnpm@10.28.0",
                "--filter",
                "@agent-room/web",
                "test:session",
            ],
        )
        environment = run.call_args.kwargs["env"]
        self.assertEqual(environment["AGENT_ROOM_E2E_USERNAME"], "developer")
        self.assertEqual(environment["AGENT_ROOM_E2E_PASSWORD"], "test-password")


if __name__ == "__main__":
    unittest.main()
