"""运维停用网络 Agent（ADR 0010）：令牌立即作废，控制面的定时清理随后替它离开所有房间。"""

from __future__ import annotations

import uuid

MAX_NAME_CHARACTERS = 64


class NetworkAgentTargetError(ValueError):
    """要停用的网络 Agent 写得不对。"""


def disable_statement(target: str) -> str:
    """停用一个网络 Agent 的 SQL。

    `target` 可以是网络 Agent ID、它的 Agent ID，或还没停用的网络 Agent 的名字（不分大小写）。
    返回的每一行是被停用的“ID 名字”。
    """
    value = target.strip()
    try:
        identifier = str(uuid.UUID(value))
    except ValueError:
        condition = f"lower(display_name) = lower({_literal(value)})"
    else:
        condition = f"(id = '{identifier}' OR agent_id = '{identifier}')"
    # 没建好实例的从没进过房间，停用时就记为已离开；与控制面的停用保持同一语义。
    return (
        "UPDATE agent_room.network_agent "
        "SET status = 'disabled', "
        "disabled_at = greatest(created_at, now()), "
        "rooms_left_at = CASE WHEN agent_instance_id IS NULL "
        "THEN greatest(created_at, now()) END "
        f"WHERE {condition} AND status <> 'disabled' "
        "RETURNING id::text || ' ' || display_name"
    )


def _literal(name: str) -> str:
    if not name or len(name) > MAX_NAME_CHARACTERS:
        raise NetworkAgentTargetError("网络 Agent 要写 ID，或 1 到 64 个字符的名字。")
    if any(ord(character) < 32 or character in "\\\x7f" for character in name):
        raise NetworkAgentTargetError("网络 Agent 的名字里不能有控制字符或反斜杠。")
    return "'" + name.replace("'", "''") + "'"
