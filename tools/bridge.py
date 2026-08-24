#!/usr/bin/env python3
"""为本地 Bridge 注入非敏感开发配置并启动设备授权流程。"""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
from typing import Final


ROOT: Final = Path(__file__).resolve().parent.parent
ENV_FILE: Final = ROOT / ".env.local"


def runtime_environment() -> dict[str, str]:
    if not ENV_FILE.is_file():
        raise RuntimeError("缺少 .env.local；请先运行 just dev-up。")

    environment = os.environ.copy()
    environment.update(
        {
            "AGENT_ROOM_CONTROL_PLANE_URL": "http://127.0.0.1:8090/",
            "AGENT_ROOM_MATRIX_BASE_URL": "http://127.0.0.1:18008",
            "AGENT_ROOM_OIDC_ISSUER_URL": (
                "http://127.0.0.1:18080/realms/agent-room"
            ),
            "AGENT_ROOM_OIDC_DEVICE_CLIENT_ID": "agent-room-bridge",
            "AGENT_ROOM_BRIDGE_REQUEST_TIMEOUT_MS": "10000",
            "AGENT_ROOM_BRIDGE_AUTHORIZATION_TIMEOUT_MS": "600000",
            "AGENT_ROOM_BRIDGE_REFRESH_LEAD_MS": "120000",
            "AGENT_ROOM_BRIDGE_RECONNECT_INITIAL_MS": "1000",
            "AGENT_ROOM_BRIDGE_RECONNECT_MAXIMUM_MS": "60000",
            "AGENT_ROOM_BRIDGE_DATA_DIR": str(ROOT / ".local" / "bridge"),
            "AGENT_ROOM_BRIDGE_IMPORT_OIDC_PROFILE": "false",
        }
    )
    return environment


def main() -> int:
    try:
        environment = runtime_environment()
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 2

    completed = subprocess.run(
        ["cargo", "run", "-p", "agent-room-bridge"],
        cwd=ROOT,
        env=environment,
        check=False,
    )
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
