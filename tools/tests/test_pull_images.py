from __future__ import annotations

from pathlib import Path
import re
import tempfile
import unittest

from tools.pull_images import (
    RETRY_DELAYS_SECONDS,
    ROOT,
    PullFailure,
    compose_images,
    main,
    pull,
    pull_all,
)


WORKFLOWS = ROOT / ".github" / "workflows"
JOB_HEADING = re.compile(r"^  ([a-z0-9-]+):$")
# 这些步骤会起 infra/compose/compose.yaml 里的容器（无头验收、负载测量和封闭测试矩阵经 vertical.py 起）。
STARTS_DEV_CONTAINERS = re.compile(
    r"dev-infra\.ps1 up|compose\.yaml up|tools/headless_acceptance\.py|tools/network_agent_load\.py|tools/closed_test\.py matrix"
)


def jobs(workflow: Path) -> dict[str, list[str]]:
    lines = workflow.read_text(encoding="utf-8").splitlines()
    start = lines.index("jobs:") + 1
    found: dict[str, list[str]] = {}
    current: str | None = None
    for line in lines[start:]:
        heading = JOB_HEADING.match(line)
        if heading:
            current = heading.group(1)
            found[current] = []
        elif current is not None:
            found[current].append(line)
    return found


def first_index(lines: list[str], pattern: re.Pattern[str] | str) -> int | None:
    for index, line in enumerate(lines):
        if (pattern.search(line) if isinstance(pattern, re.Pattern) else pattern in line):
            return index
    return None


class FakeDocker:
    def __init__(self, *, local: set[str] = frozenset(), failures: dict[str, int] | None = None) -> None:
        self.local = set(local)
        self.failures = dict(failures or {})
        self.pulls: list[str] = []
        self.sleeps: list[float] = []

    def probe(self, arguments: list[str]) -> int:
        return 0 if arguments[-1] in self.local else 1

    def run(self, arguments: list[str]) -> int:
        image = arguments[-1]
        self.pulls.append(image)
        if self.failures.get(image, 0) > 0:
            self.failures[image] -= 1
            return 1
        self.local.add(image)
        return 0

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)

    def options(self) -> dict[str, object]:
        return {"run": self.run, "probe": self.probe, "sleep": self.sleep}


class ComposeImagesTests(unittest.TestCase):
    def test_读出开发环境每个服务的镜像(self) -> None:
        images = compose_images(ROOT / "infra" / "compose" / "compose.yaml")

        self.assertIn("postgres:18.6-alpine", images)
        self.assertIn("quay.io/keycloak/keycloak:26.7.2", images)
        self.assertEqual(len(images), len(set(images)))
        self.assertTrue(all("$" not in image for image in images))

    def test_联邦环境重复的镜像只拉一次(self) -> None:
        images = compose_images(ROOT / "infra" / "federation" / "compose.yaml")

        self.assertEqual(images.count("matrixdotorg/synapse:v1.159.0"), 1)
        self.assertEqual(images.count("postgres:18.6-alpine"), 1)

    def test_只拉点名的服务(self) -> None:
        images = compose_images(ROOT / "infra" / "compose" / "compose.yaml", ["postgres", "content-scanner"])

        self.assertEqual(images, ["postgres:18.6-alpine", "clamav/clamav:1.5.4"])

    def test_点了没有的服务或镜像带变量就报错(self) -> None:
        with self.assertRaisesRegex(PullFailure, "nope"):
            compose_images(ROOT / "infra" / "compose" / "compose.yaml", ["nope"])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "compose.yaml"
            path.write_text("services:\n  app:\n    image: ${IMAGE}\n", encoding="utf-8")
            with self.assertRaisesRegex(PullFailure, "变量"):
                compose_images(path)

    def test_不把别的顶层键下面的内容当服务(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "compose.yaml"
            path.write_text(
                "x-logging: &logging\n  driver: local\n"
                "services:\n  # 注释\n  app:\n    image: 'busybox:1.37' # 固定版本\n"
                "volumes:\n  data:\n    image: not-a-service\n",
                encoding="utf-8",
            )
            self.assertEqual(compose_images(path), ["busybox:1.37"])


class PullTests(unittest.TestCase):
    def test_本地已有就不拉(self) -> None:
        docker = FakeDocker(local={"postgres:18.6-alpine"})

        pull("postgres:18.6-alpine", **docker.options())

        self.assertEqual(docker.pulls, [])

    def test_失败后等一会儿再拉(self) -> None:
        docker = FakeDocker(failures={"quay.io/keycloak/keycloak:26.7.2": 2})

        pull("quay.io/keycloak/keycloak:26.7.2", **docker.options())

        self.assertEqual(len(docker.pulls), 3)
        self.assertEqual(docker.sleeps, list(RETRY_DELAYS_SECONDS[:2]))

    def test_都失败才报错(self) -> None:
        docker = FakeDocker(failures={"caddy:2.11.4-alpine": 99})

        with self.assertRaisesRegex(PullFailure, "caddy"):
            pull("caddy:2.11.4-alpine", **docker.options())

        self.assertEqual(len(docker.pulls), len(RETRY_DELAYS_SECONDS) + 1)
        self.assertEqual(docker.sleeps, list(RETRY_DELAYS_SECONDS))

    def test_一个拉不下来也等别的拉完再报错(self) -> None:
        docker = FakeDocker(failures={"a:1": 99})

        with self.assertRaisesRegex(PullFailure, "a:1"):
            pull_all(["a:1", "b:1"], **docker.options())

        self.assertIn("b:1", docker.local)

    def test_命令行点了没有的服务返回失败(self) -> None:
        self.assertEqual(main(["infra/compose/compose.yaml", "nope"]), 1)


class WorkflowWiringTests(unittest.TestCase):
    def test_起开发环境容器的作业先预拉镜像(self) -> None:
        checked = 0
        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            for name, lines in jobs(workflow).items():
                start = first_index(lines, STARTS_DEV_CONTAINERS)
                if start is None:
                    continue
                checked += 1
                with self.subTest(workflow=workflow.name, job=name):
                    pulled = first_index(lines, "python tools/pull_images.py infra/compose/compose.yaml")
                    self.assertIsNotNone(pulled)
                    self.assertLess(pulled, start)
        self.assertGreaterEqual(checked, 5)

    def test_联邦验收先预拉镜像(self) -> None:
        lines = jobs(WORKFLOWS / "federation.yml")["federation"]

        pulled = first_index(lines, "python tools/pull_images.py infra/federation/compose.yaml")
        self.assertIsNotNone(pulled)
        self.assertLess(pulled, first_index(lines, "tools/federation.py bootstrap"))


if __name__ == "__main__":
    unittest.main()
