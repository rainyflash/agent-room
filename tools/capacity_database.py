#!/usr/bin/env python3
"""在隔离 PostgreSQL 中执行任务 39 的真实数据库容量场景。"""

from __future__ import annotations

from datetime import UTC, datetime
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Final

if __package__:
    from .capacity import git_revision, write_json
    from .database import (
        ENV_FILE,
        TEST_DATABASE,
        database_url,
        drop_test_database,
        ensure_runtime_role,
        read_environment,
        required_value,
        reset_test_database,
    )
else:
    from capacity import git_revision, write_json
    from database import (
        ENV_FILE,
        TEST_DATABASE,
        database_url,
        drop_test_database,
        ensure_runtime_role,
        read_environment,
        required_value,
        reset_test_database,
    )


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT: Final = ROOT / "artifacts" / "capacity" / "database-report.json"
OBSERVATION_PREFIX: Final = "CAPACITY_OBSERVATION="


class DatabaseCapacityFailure(RuntimeError):
    """表示真实数据库容量场景没有产出可信观察。"""


def parse_observation(output: str) -> dict[str, object]:
    matches = [
        line.split(OBSERVATION_PREFIX, maxsplit=1)[1]
        for line in output.splitlines()
        if OBSERVATION_PREFIX in line
    ]
    if len(matches) != 1:
        raise DatabaseCapacityFailure("数据库容量测试没有产出唯一观察记录。")
    try:
        value = json.loads(matches[0])
    except json.JSONDecodeError as error:
        raise DatabaseCapacityFailure("数据库容量观察不是有效 JSON。") from error
    if not isinstance(value, dict):
        raise DatabaseCapacityFailure("数据库容量观察必须是 JSON 对象。")
    return value


def capacity_test_command() -> list[str]:
    """使用发布构建执行性能预算，拒绝覆盖率或调试插桩污染结果。"""

    return [
        "cargo",
        "test",
        "--release",
        "-p",
        "agent-room-postgres-adapter",
        "--test",
        "capacity",
        "--",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]


def run() -> dict[str, object]:
    values = read_environment(ENV_FILE)
    migration_password = required_value(values, "AGENT_ROOM_DB_PASSWORD")
    runtime_password = required_value(values, "AGENT_ROOM_DB_RUNTIME_PASSWORD")
    ensure_runtime_role(runtime_password)
    reset_test_database()

    environment = os.environ.copy()
    environment["AGENT_ROOM_TEST_MIGRATION_DATABASE_URL"] = database_url(
        "agent_room", migration_password, TEST_DATABASE
    )
    environment["AGENT_ROOM_TEST_RUNTIME_DATABASE_URL"] = database_url(
        "agent_room_runtime", runtime_password, TEST_DATABASE
    )
    environment["RUST_BACKTRACE"] = "0"
    try:
        result = subprocess.run(
            capacity_test_command(),
            cwd=ROOT,
            env=environment,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        combined = result.stdout + result.stderr
        if result.returncode != 0:
            raise DatabaseCapacityFailure(
                "真实 PostgreSQL 容量测试失败：\n" + combined[-4_000:]
            )
        metrics = parse_observation(combined)
    finally:
        drop_test_database()

    report: dict[str, object] = {
        "schemaVersion": 1,
        "scenario": "database_directory_and_allocation",
        "evidenceLevel": "real_postgresql",
        "generatedAt": datetime.now(UTC).isoformat(),
        "revision": git_revision(),
        "passed": True,
        "releaseGateEligible": True,
        "topology": {
            "database": "PostgreSQL 18 isolated capacity database",
            "agents": 10_000,
            "onlineInstances": 1_000,
            "lobbyMembers": 250,
        },
        "metrics": metrics,
    }
    write_json(REPORT, report)
    return report


def main() -> int:
    try:
        report = run()
        print(f"数据库容量报告：{REPORT}")
        print(json.dumps(report["metrics"], ensure_ascii=False, indent=2))
        return 0
    except (DatabaseCapacityFailure, RuntimeError) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
