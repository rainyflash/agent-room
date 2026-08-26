#!/usr/bin/env python3
"""准备本地私有对象桶并运行真实 SeaweedFS 兼容性测试。"""

from __future__ import annotations

import argparse
import os
from datetime import UTC, datetime
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Final, Sequence

if __package__:
    from .capacity import git_revision, require_git_revision, write_json
else:
    from capacity import git_revision, require_git_revision, write_json


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"
SAFE_SECRET: Final = re.compile(r"^[A-Za-z0-9._~-]{3,256}$")
REPORT: Final = ROOT / "artifacts" / "capacity" / "content-report.json"
OBSERVATION_PREFIX: Final = "CAPACITY_CONTENT_OBSERVATION="


def read_environment() -> dict[str, str]:
    if not ENV_FILE.is_file():
        raise RuntimeError("缺少 .env.local；请先运行 just dev-up。")

    values: dict[str, str] = {}
    for raw_line in ENV_FILE.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key:
            raise RuntimeError(".env.local 包含无效配置行。")
        values[key] = value
    return values


def required_value(values: dict[str, str], name: str) -> str:
    value = values.get(name)
    if value is None or SAFE_SECRET.fullmatch(value) is None:
        raise RuntimeError(f".env.local 中的 {name} 缺失或格式非法。")
    return value


def run(
    command: list[str], environment: dict[str, str], *, capture: bool = False
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=capture,
        text=capture,
        encoding="utf-8" if capture else None,
        errors="replace" if capture else None,
    )
    if result.returncode != 0:
        raise RuntimeError(f"命令执行失败，退出码为 {result.returncode}。")
    return result


def parse_capacity_observation(output: str) -> dict[str, object]:
    matches = [
        line.split(OBSERVATION_PREFIX, maxsplit=1)[1]
        for line in output.splitlines()
        if OBSERVATION_PREFIX in line
    ]
    if len(matches) != 1:
        raise RuntimeError("对象容量测试没有产出唯一观察记录。")
    value = json.loads(matches[0])
    if not isinstance(value, dict):
        raise RuntimeError("对象容量观察必须是 JSON 对象。")
    return value


def parse_args(arguments: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    parse_args(arguments)
    try:
        revision = git_revision()
        values = read_environment()
        environment = os.environ.copy()
        environment.update(
            {
                "AGENT_ROOM_TEST_S3_ENDPOINT": "http://127.0.0.1:18333",
                "AGENT_ROOM_TEST_S3_BUCKET": "agent-room-content",
                "AGENT_ROOM_TEST_S3_REGION": "us-east-1",
                "AGENT_ROOM_TEST_S3_ACCESS_KEY": required_value(
                    values, "S3_ACCESS_KEY"
                ),
                "AGENT_ROOM_TEST_S3_SECRET_KEY": required_value(
                    values, "S3_SECRET_KEY"
                ),
            }
        )
        # 复用唯一的基础设施种子入口，避免测试脚本复制桶创建和凭据传递逻辑。
        run(
            [
                "node",
                "tools/run-powershell.mjs",
                "tools/dev-infra.ps1",
                "seed",
            ],
            environment,
        )
        result = run(
            [
                "cargo",
                "test",
                "-p",
                "agent-room-content-adapter",
                "--test",
                "seaweedfs",
                "--",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ],
            environment,
            capture=True,
        )
        output = (result.stdout or "") + (result.stderr or "")
        metrics = parse_capacity_observation(output)
        require_git_revision(revision)
        write_json(
            REPORT,
            {
                "schemaVersion": 1,
                "scenario": "content_25_mib_concurrency",
                "evidenceLevel": "real_s3_compatible_store",
                "generatedAt": datetime.now(UTC).isoformat(),
                "revision": revision,
                "passed": True,
                "releaseGateEligible": True,
                "topology": "SeaweedFS S3 compatibility path through the production adapter",
                "metrics": metrics,
            },
        )
        print(f"对象容量报告：{REPORT}")
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
