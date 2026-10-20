from __future__ import annotations

import json
from pathlib import Path
import re
import unittest

from tools.bump_release_version import JSON_VERSION_FILES, workspace_version


ROOT = Path(__file__).resolve().parents[2]
SERVER_JSON = ROOT / "server.json"
DIRECTORIES = ROOT / "docs" / "mcp-directories.md"
REMOTE_MCP = ROOT / "apps" / "control-plane" / "src" / "features" / "network_agents"


def remote_tools() -> dict[str, str]:
    """从远程 MCP 的源码读出每个工具的名字和标题。"""
    sources = [REMOTE_MCP / "mcp.rs", *sorted((REMOTE_MCP / "mcp").glob("*.rs"))]
    tools: dict[str, str] = {}
    for source in sources:
        text = source.read_text(encoding="utf-8")
        for block in re.findall(r"#\[tool\((.*?)\n    \)\]", text, flags=re.DOTALL):
            name = re.search(r'name = "([a-z_]+)"', block)
            title = re.search(r'title = "([^"]+)"', block)
            if name and title:
                tools[name.group(1)] = title.group(1)
    return tools


class McpServerJsonTests(unittest.TestCase):
    def setUp(self) -> None:
        self.server = json.loads(SERVER_JSON.read_text(encoding="utf-8"))

    def test_官方目录要的字段都对(self) -> None:
        self.assertEqual(
            self.server["$schema"],
            "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
        )
        # 名字的前缀要和发布时登录的 GitHub 账号一致。
        self.assertEqual(self.server["name"], "io.github.rainyflash/agent-room")
        self.assertLessEqual(len(self.server["description"]), 100)
        self.assertEqual(
            self.server["remotes"],
            [{"type": "streamable-http", "url": "https://api.agentroom.chat/mcp"}],
        )
        self.assertEqual(
            self.server["repository"]["url"], "https://github.com/rainyflash/agent-room"
        )
        self.assertTrue(self.server["icons"])
        for icon in self.server["icons"]:
            self.assertTrue(icon["src"].startswith("https://"), icon)

    def test_版本跟着发布走(self) -> None:
        self.assertIn(Path("server.json"), JSON_VERSION_FILES)
        self.assertEqual(self.server["version"], workspace_version(ROOT))

    def test_目录说明里的工具和代码一致(self) -> None:
        tools = remote_tools()
        listed = dict(
            re.findall(
                r"^\s*\| `(agent_room_[a-z_]+)`\s*\| ([^|]+?)\s*\|$",
                DIRECTORIES.read_text(encoding="utf-8"),
                flags=re.MULTILINE,
            )
        )

        self.assertEqual(len(tools), 16)
        self.assertEqual(listed, tools)

    def test_一句话和_server_json_一样(self) -> None:
        self.assertIn(self.server["description"], DIRECTORIES.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
