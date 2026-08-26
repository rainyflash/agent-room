#!/usr/bin/env python3
"""在当前 Git 提交的干净快照中执行 Task 44 开源验收。"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tempfile
import time
from typing import Final


ROOT: Final = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT: Final = ROOT / "artifacts" / "oss" / "task-44-acceptance.json"


class AcceptanceError(RuntimeError):
    """表示干净快照中的开源或自托管流程失败。"""


@dataclass(frozen=True, slots=True)
class StepResult:
    name: str
    duration_ms: int
    status: str


def run_acceptance(output: Path) -> dict[str, object]:
    revision = _capture(["git", "rev-parse", "HEAD"], ROOT).strip()
    steps: list[StepResult] = []
    with tempfile.TemporaryDirectory(prefix="agent-room-task-44-") as directory:
        temporary = Path(directory)
        archive = temporary / "source.tar"
        checkout = temporary / "source"
        checkout.mkdir()
        _run_step(
            steps,
            "export-clean-git-snapshot",
            ["git", "archive", "--format=tar", f"--output={archive}", revision],
            ROOT,
        )
        with tarfile.open(archive, "r") as source:
            source.extractall(checkout, filter="data")

        _run_step(steps, "bootstrap-clean-workspace", ["node", "tools/bootstrap.mjs"], checkout)
        _run_step(
            steps,
            "verify-contributor-environment",
            ["node", "tools/bootstrap.mjs", "--check"],
            checkout,
        )
        _run_step(
            steps,
            "verify-oss-surface",
            [sys.executable, "tools/open_source.py"],
            checkout,
        )
        _run_step(
            steps,
            "verify-license-inventory",
            [sys.executable, "tools/license_inventory.py", "check"],
            checkout,
        )
        _run_step(
            steps,
            "verify-self-host-tests",
            [
                sys.executable,
                "-m",
                "unittest",
                "tools.tests.test_self_host",
                "tools.tests.test_prodops",
            ],
            checkout,
        )

        acceptance = checkout / ".acceptance"
        config = acceptance / "deployment.json"
        state = acceptance / "state"
        _run_step(
            steps,
            "generate-default-self-host-config",
            [
                sys.executable,
                "tools/self_host.py",
                "init",
                "--domain",
                "room.example.com",
                "--email",
                "operator@example.com",
                "--output",
                str(config),
            ],
            checkout,
        )
        _run_step(
            steps,
            "render-and-validate-production-compose",
            [
                sys.executable,
                "tools/production.py",
                "validate",
                "--config",
                str(config),
                "--state-dir",
                str(state),
            ],
            checkout,
        )
        document = json.loads(config.read_text(encoding="utf-8"))

    report: dict[str, object] = {
        "schemaVersion": 1,
        "revision": revision,
        "result": "pass",
        "environment": {
            "system": platform.system(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "selfHost": {
            "databaseMode": document["database"]["mode"],
            "objectStoreMode": document["objectStore"]["mode"],
            "credentialsInConfig": False,
            "composeValidated": True,
        },
        "steps": [asdict(step) for step in steps],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    return report


def _run_step(
    results: list[StepResult], name: str, command: list[str], working_directory: Path
) -> None:
    started = time.monotonic()
    completed = subprocess.run(
        command,
        cwd=working_directory,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    duration_ms = round((time.monotonic() - started) * 1000)
    if completed.returncode != 0:
        detail = "\n".join(part.strip() for part in (completed.stdout, completed.stderr) if part.strip())
        raise AcceptanceError(f"{name} 失败（exit {completed.returncode}）：\n{detail}")
    results.append(StepResult(name, duration_ms, "pass"))
    print(f"通过：{name}（{duration_ms} ms）")


def _capture(command: list[str], working_directory: Path) -> str:
    completed = subprocess.run(
        command,
        cwd=working_directory,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        raise AcceptanceError(completed.stderr.strip() or "Git 命令失败。")
    return completed.stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    arguments = parser.parse_args()
    try:
        report = run_acceptance(arguments.output)
    except (AcceptanceError, OSError, tarfile.TarError) as error:
        print(str(error), file=sys.stderr)
        return 1
    print(f"Task 44 干净快照验收通过：{len(report['steps'])} 个步骤；{arguments.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
