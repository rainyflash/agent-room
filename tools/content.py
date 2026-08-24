#!/usr/bin/env python3
"""在隔离数据库中验证真实内容存储与扫描管线。"""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
from typing import Final

import database
import object_store


ROOT: Final = Path(__file__).resolve().parent.parent


def ensure_content_bucket() -> None:
    result = subprocess.run(
        [
            "node",
            "tools/run-powershell.mjs",
            "tools/dev-infra.ps1",
            "seed",
        ],
        cwd=ROOT,
        env=os.environ.copy(),
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError("本地内容桶初始化失败。")


def test_environment(values: dict[str, str]) -> dict[str, str]:
    return {
        "AGENT_ROOM_TEST_S3_ENDPOINT": "http://127.0.0.1:18333",
        "AGENT_ROOM_TEST_S3_BUCKET": "agent-room-content",
        "AGENT_ROOM_TEST_S3_REGION": "us-east-1",
        "AGENT_ROOM_TEST_S3_ACCESS_KEY": object_store.required_value(
            values, "S3_ACCESS_KEY"
        ),
        "AGENT_ROOM_TEST_S3_SECRET_KEY": object_store.required_value(
            values, "S3_SECRET_KEY"
        ),
        "AGENT_ROOM_TEST_CLAMAV_ADDRESS": "127.0.0.1:13310",
    }


def main() -> int:
    try:
        values = database.read_environment(database.ENV_FILE)
        ensure_content_bucket()
        database.run_in_test_database(
            values,
            [
                [
                    "cargo",
                    "test",
                    "-p",
                    "agent-room-control-plane",
                    "--test",
                    "content_real_flow",
                    "--",
                    "--ignored",
                    "--test-threads=1",
                ]
            ],
            additional_environment=test_environment(values),
        )
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
