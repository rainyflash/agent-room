#!/usr/bin/env python3
"""以迁移账号管理 PostgreSQL，并以运行时账号执行真实仓储测试。"""

from __future__ import annotations

import argparse
from collections.abc import Callable
import os
from pathlib import Path
import re
import subprocess
import sys
from typing import Final
from urllib.parse import quote


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"
COMPOSE_FILE: Final = ROOT / "infra" / "compose" / "compose.yaml"
PROJECT_NAME: Final = "agent-room-dev"
TEST_DATABASE: Final = "agent_room_repository_test"
SAFE_SECRET: Final = re.compile(r"^[A-Za-z0-9._~-]{16,256}$")


def read_environment(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise RuntimeError("缺少 .env.local；请先运行 just dev-up。")

    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
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
    if not value:
        raise RuntimeError(f".env.local 缺少 {name}。")
    if not SAFE_SECRET.fullmatch(value):
        raise RuntimeError(f"{name} 含有不受支持的字符或长度不合法。")
    return value


def compose_psql(sql: str) -> None:
    command = [
        "docker",
        "compose",
        "--project-name",
        PROJECT_NAME,
        "--env-file",
        str(ENV_FILE),
        "--file",
        str(COMPOSE_FILE),
        "exec",
        "-T",
        "postgres",
        "psql",
        "--set=ON_ERROR_STOP=1",
        "--username=agent_room_bootstrap",
        "--dbname=postgres",
    ]
    result = subprocess.run(
        command,
        cwd=ROOT,
        input=sql,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError("PostgreSQL 管理命令失败；请确认 just dev-up 已完成。")


def ensure_runtime_role(runtime_password: str) -> None:
    # 密码只通过标准输入传给 psql，避免出现在命令行和日志中。
    sql = f"""
SELECT format(
    'CREATE ROLE agent_room_runtime LOGIN PASSWORD %L NOSUPERUSER NOCREATEDB NOCREATEROLE',
    '{runtime_password}'
)
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'agent_room_runtime')
\\gexec
ALTER ROLE agent_room_runtime
    LOGIN PASSWORD '{runtime_password}' NOSUPERUSER NOCREATEDB NOCREATEROLE;
GRANT CONNECT ON DATABASE agent_room TO agent_room_runtime;
"""
    compose_psql(sql)


def database_url(username: str, password: str, database: str) -> str:
    encoded_user = quote(username, safe="")
    encoded_password = quote(password, safe="")
    encoded_database = quote(database, safe="")
    return (
        f"postgresql://{encoded_user}:{encoded_password}"
        f"@127.0.0.1:55432/{encoded_database}?sslmode=disable"
    )


def run(command: list[str], environment: dict[str, str]) -> None:
    result = subprocess.run(command, cwd=ROOT, env=environment, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"命令执行失败，退出码为 {result.returncode}。")


def reset_test_database() -> None:
    if TEST_DATABASE != "agent_room_repository_test":
        raise RuntimeError("拒绝操作未经审计的测试数据库名。")

    compose_psql(
        f"""
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE datname = '{TEST_DATABASE}' AND pid <> pg_backend_pid();
DROP DATABASE IF EXISTS {TEST_DATABASE} WITH (FORCE);
CREATE DATABASE {TEST_DATABASE} OWNER agent_room;
REVOKE CONNECT ON DATABASE {TEST_DATABASE} FROM PUBLIC;
GRANT CONNECT ON DATABASE {TEST_DATABASE} TO agent_room_runtime;
"""
    )


def drop_test_database() -> None:
    if TEST_DATABASE != "agent_room_repository_test":
        raise RuntimeError("拒绝操作未经审计的测试数据库名。")

    compose_psql(
        f"""
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE datname = '{TEST_DATABASE}' AND pid <> pg_backend_pid();
DROP DATABASE IF EXISTS {TEST_DATABASE} WITH (FORCE);
"""
    )


def migrate(values: dict[str, str]) -> None:
    migration_password = required_value(values, "AGENT_ROOM_DB_PASSWORD")
    runtime_password = required_value(values, "AGENT_ROOM_DB_RUNTIME_PASSWORD")
    ensure_runtime_role(runtime_password)

    environment = os.environ.copy()
    environment["AGENT_ROOM_MIGRATION_DATABASE_URL"] = database_url(
        "agent_room", migration_password, "agent_room"
    )
    run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "agent-room-postgres-adapter",
            "--bin",
            "migrate",
        ],
        environment,
    )


def run_in_test_database(
    values: dict[str, str],
    commands: list[list[str]],
    additional_environment: dict[str, str] | None = None,
) -> None:
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
    environment.update(additional_environment or {})
    try:
        for command in commands:
            run(command, environment)
    finally:
        drop_test_database()


def test(values: dict[str, str]) -> None:
    run_in_test_database(
        values,
        [
            [
                "cargo",
                "test",
                "-p",
                "agent-room-postgres-adapter",
                "--tests",
                "--",
                "--ignored",
                "--test-threads=1",
            ]
        ],
    )


def coverage(values: dict[str, str]) -> None:
    run_in_test_database(
        values,
        [
            ["cargo", "llvm-cov", "clean", "--workspace"],
            [
                "cargo",
                "llvm-cov",
                "--workspace",
                "--all-features",
                "--no-report",
            ],
            [
                "cargo",
                "llvm-cov",
                "--all-features",
                "-p",
                "agent-room-postgres-adapter",
                "--tests",
                "--no-report",
                "--",
                "--ignored",
                "--test-threads=1",
            ],
            [sys.executable, "tools/matrix.py", "coverage"],
            [
                "cargo",
                "llvm-cov",
                "report",
                "--summary-only",
                "--fail-under-lines",
                "60",
            ],
        ],
    )


ACTIONS: Final[dict[str, Callable[[dict[str, str]], None]]] = {
    "migrate": migrate,
    "test": test,
    "coverage": coverage,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=tuple(ACTIONS))
    arguments = parser.parse_args()

    try:
        values = read_environment(ENV_FILE)
        ACTIONS[arguments.action](values)
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
