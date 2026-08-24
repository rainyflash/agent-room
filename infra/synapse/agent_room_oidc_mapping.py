"""Agent Room 的确定性 Synapse OIDC 用户映射器。"""

from __future__ import annotations

from collections.abc import Mapping
from hashlib import sha256


_HASH_BYTES = 16


def localpart_from_identity(issuer: str, subject: str) -> str:
    """用稳定 OIDC 主体生成不泄露原始标识的 Matrix localpart。"""

    digest = sha256(issuer.encode("utf-8") + b"\0" + subject.encode("utf-8")).hexdigest()
    return f"user-{digest[: _HASH_BYTES * 2]}"


def _required_subject(userinfo: Mapping[str, object]) -> str:
    subject = userinfo.get("sub")
    if not isinstance(subject, str) or not subject or subject != subject.strip():
        raise RuntimeError("OIDC sub 必须是非空且无首尾空白的字符串。")
    return subject


def _optional_text(userinfo: Mapping[str, object], claim: str) -> str | None:
    value = userinfo.get(claim)
    if not isinstance(value, str):
        return None
    normalized = value.strip()
    return normalized or None


class AgentRoomOidcMappingProvider:
    """与控制平面使用同一 issuer + sub 映射规则。"""

    def __init__(self, config: Mapping[str, object], module_api: object) -> None:
        del module_api
        issuer = config.get("issuer")
        if not isinstance(issuer, str):
            raise RuntimeError("映射器缺少已验证的 issuer 配置。")
        self._issuer = issuer

    @staticmethod
    def parse_config(config: Mapping[str, object]) -> Mapping[str, object]:
        issuer = config.get("issuer")
        if not isinstance(issuer, str) or not issuer or issuer != issuer.strip():
            raise ValueError("issuer 必须是非空且无首尾空白的字符串。")
        return {"issuer": issuer}

    def get_remote_user_id(self, userinfo: Mapping[str, object]) -> str:
        return _required_subject(userinfo)

    async def map_user_attributes(
        self,
        userinfo: Mapping[str, object],
        token: Mapping[str, object],
        failures: int,
    ) -> Mapping[str, object]:
        del token
        if failures != 0:
            raise RuntimeError("确定性 Matrix 用户名已被占用，拒绝生成带后缀的错误身份。")

        subject = _required_subject(userinfo)
        display_name = _optional_text(userinfo, "name") or _optional_text(
            userinfo, "preferred_username"
        )
        email = _optional_text(userinfo, "email")

        return {
            "localpart": localpart_from_identity(self._issuer, subject),
            "display_name": display_name,
            "emails": [email] if email is not None else [],
            "picture": _optional_text(userinfo, "picture"),
            "confirm_localpart": False,
        }

    async def get_extra_attributes(
        self,
        userinfo: Mapping[str, object],
        token: Mapping[str, object],
    ) -> Mapping[str, object]:
        del userinfo, token
        return {}
