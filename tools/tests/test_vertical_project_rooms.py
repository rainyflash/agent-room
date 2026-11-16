import socket
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import urlopen

from tools import vertical
from tools.vertical import VerticalFailure
from tools.vertical_project_rooms import RAW_PATH, FakeGithub, claim_line

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


if __name__ == "__main__":
    unittest.main()
