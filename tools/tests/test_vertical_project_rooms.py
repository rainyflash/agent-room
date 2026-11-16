from pathlib import Path
import re
import socket
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import urlopen

from tools import vertical
from tools.vertical import VerticalFailure
from tools.vertical_project_rooms import (
    LIVE_SPEC,
    LIVE_STEPS,
    RAW_PATH,
    FakeGithub,
    LiveLine,
    claim_line,
    has_live_panel,
    live_browser_environment,
    live_lines,
    require_said_by_agent,
)

REPOSITORY = "vertical-octo/hello-1a2b"
SHARE_URL = "https://agentroom.localhost/r/K7p3Qx"


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def pending(**overrides: object) -> dict[str, object]:
    data: dict[str, object] = {
        "repository": REPOSITORY,
        "status": "pending",
        "shareUrl": SHARE_URL,
        "agentsLine": f"Agent Room: this project's room is {SHARE_URL}.",
        "readmeBadge": f"[![Agent Room](https://agentroom.localhost/gh/{REPOSITORY}/badge.svg)]({SHARE_URL})",
    }
    data.update(overrides)
    return data


class FakeGithubTests(unittest.TestCase):
    def test_照_raw_的路径给文件_没有的回_404_都记下来(self) -> None:
        port = free_port()
        with FakeGithub(port) as github:
            github.serve(REPOSITORY, "AGENTS.md", f"Agent Room: {SHARE_URL}\n")
            with urlopen(f"http://127.0.0.1:{port}/{REPOSITORY}/HEAD/AGENTS.md", timeout=5) as response:
                self.assertEqual(response.read().decode("utf-8"), f"Agent Room: {SHARE_URL}\n")
            with self.assertRaises(HTTPError) as missing:
                urlopen(f"http://127.0.0.1:{port}/{REPOSITORY}/HEAD/README.md", timeout=5)
            self.assertEqual(missing.exception.code, 404)
            missing.exception.close()
            self.assertEqual(
                github.paths(), [f"/{REPOSITORY}/HEAD/AGENTS.md", f"/{REPOSITORY}/HEAD/README.md"]
            )
        self.assertTrue(all(RAW_PATH.fullmatch(path) for path in github.paths()))

    def test_只认那两个文件(self) -> None:
        for path in (
            f"/{REPOSITORY}/main/AGENTS.md",
            f"/{REPOSITORY}/HEAD/CLAUDE.md",
            "/someone-else/hello/HEAD/AGENTS.md",
            f"/{REPOSITORY}/HEAD/AGENTS.md?x=1",
        ):
            self.assertIsNone(RAW_PATH.fullmatch(path), path)

    def test_端口和控制面配置用的是同一个(self) -> None:
        with patch("tools.vertical.control_plane_runtime_environment", return_value={}):
            environment = vertical.vertical_control_plane_environment({})
        self.assertEqual(
            environment["AGENT_ROOM_ACCEPTANCE_GITHUB_ADDRESS"], f"127.0.0.1:{vertical.FAKE_GITHUB_PORT}"
        )


class ClaimLineTests(unittest.TestCase):
    def test_等着的认领回分享链接和码(self) -> None:
        self.assertEqual(claim_line(pending(), REPOSITORY), (SHARE_URL, "K7p3Qx"))

    def test_那一行和徽章要指着这次认领的链接(self) -> None:
        for data in (
            pending(status="bound"),
            pending(repository="vertical-octo/other"),
            pending(shareUrl="https://agentroom.localhost/gh/vertical-octo/hello-1a2b"),
            pending(agentsLine="Agent Room: https://agentroom.localhost/r/Zzzzzz"),
            pending(readmeBadge=f"[![Agent Room](https://example.com/badge.svg)]({SHARE_URL})"),
        ):
            with self.assertRaises(VerticalFailure):
                claim_line(data, REPOSITORY)


def snapshot(**overrides: object) -> dict[str, object]:
    data: dict[str, object] = {
        "schemaVersion": 1,
        "online": {"count": 2, "atLeast": False},
        "participants": [
            {"key": "p-agent", "name": "Project helper #2316", "kind": "agent", "owner": "Local Developer"},
            {"key": "p-person", "name": "Local Developer", "kind": "person", "owner": None},
        ],
        "messages": [
            {"key": "m-1", "author": "p-person", "text": "Hi there", "sentAtUnixMs": 1},
            {"key": "m-2", "author": "p-agent", "text": "Hello from the project room (1a2b).", "sentAtUnixMs": 2},
            {"key": "m-3", "author": "p-person", "text": "", "withheld": True, "sentAtUnixMs": 3},
        ],
        "updatedAtUnixMs": 4,
    }
    data.update(overrides)
    return data


class LiveLinesTests(unittest.TestCase):
    def test_每一句配上名单里的说话人(self) -> None:
        self.assertEqual(
            live_lines(snapshot()),
            [
                LiveLine(name="Local Developer", kind="person", owner=None, text="Hi there"),
                LiveLine(
                    name="Project helper #2316",
                    kind="agent",
                    owner="Local Developer",
                    text="Hello from the project room (1a2b).",
                ),
                LiveLine(name="Local Developer", kind="person", owner=None, text=""),
            ],
        )

    def test_版本不对_说话人不在名单里都算错(self) -> None:
        stray = {"key": "m-9", "author": "p-nobody", "text": "?", "sentAtUnixMs": 9}
        for data in (snapshot(schemaVersion=2), snapshot(messages=[stray]), snapshot(participants=None)):
            with self.assertRaises(VerticalFailure):
                live_lines(data)


class SaidByAgentTests(unittest.TestCase):
    TEXT = "Hello from the project room (1a2b)."

    def line(self, **overrides: object) -> LiveLine:
        fields: dict[str, object] = {
            "name": "Project helper #2316",
            "kind": "agent",
            "owner": "Local Developer",
            "text": self.TEXT,
        }
        fields.update(overrides)
        return LiveLine(**fields)  # type: ignore[arg-type]

    def test_本机_Agent_说的_写着主人(self) -> None:
        require_said_by_agent([self.line(), self.line(text="other")], self.TEXT)

    def test_不是这个_Agent_说的或者不止一句都不算(self) -> None:
        for lines in (
            [],
            [self.line(), self.line()],
            [self.line(kind="person")],
            [self.line(kind="networkAgent")],
            [self.line(name="Project helper")],
            [self.line(name="Project helper #2316 #1111")],
            [self.line(owner=None)],
            [self.line(owner="Local Collaborator")],
        ):
            with self.assertRaises(VerticalFailure):
                require_said_by_agent(lines, self.TEXT)


class LivePanelTests(unittest.TestCase):
    def test_只认这个码的那一块(self) -> None:
        html = '<section class="share-panel share-live" hidden data-live="/r/K7p3Qx/live" data-x="1">'
        self.assertTrue(has_live_panel(html, "K7p3Qx"))
        self.assertFalse(has_live_panel(html, "Zzzzzz"))
        self.assertFalse(has_live_panel('<a href="/r/K7p3Qx">', "K7p3Qx"))


class LiveBrowserEnvironmentTests(unittest.TestCase):
    def test_网页用例读的变量都给了_对步子的文件是绝对路径(self) -> None:
        environment = live_browser_environment(
            {"SEED_ADMIN_PASSWORD": "seed"}, project_catalog="project", former_public_room="former"
        )
        spec = (vertical.ROOT / "apps" / "web" / "e2e-vertical" / LIVE_SPEC).read_text(encoding="utf-8")
        wanted = set(re.findall(r"process\.env\.(AGENT_ROOM_[A-Z0-9_]+)", spec))
        self.assertEqual(
            wanted,
            {
                "AGENT_ROOM_E2E_USERNAME",
                "AGENT_ROOM_E2E_PASSWORD",
                "AGENT_ROOM_VERTICAL_PROJECT_LIVE_ROOM",
                "AGENT_ROOM_VERTICAL_PROJECT_LIVE_FORMER_PUBLIC_ROOM",
                *(f"AGENT_ROOM_VERTICAL_PROJECT_LIVE_{name}" for name, _ in LIVE_STEPS),
            },
        )
        self.assertTrue(wanted <= environment.keys())
        self.assertEqual(environment["AGENT_ROOM_E2E_USERNAME"], "developer")
        self.assertEqual(environment["AGENT_ROOM_VERTICAL_PROJECT_LIVE_ROOM"], "project")
        for name, path in LIVE_STEPS:
            self.assertTrue(Path(environment[f"AGENT_ROOM_VERTICAL_PROJECT_LIVE_{name}"]).is_absolute())
            self.assertEqual(path.parent, vertical.VERTICAL_ROOT)


if __name__ == "__main__":
    unittest.main()
