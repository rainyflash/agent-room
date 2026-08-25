#!/usr/bin/env python3
"""扫描日志、指标、错误、审计和崩溃输出中的高敏值。"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
from typing import Final, Iterable, Sequence


JWT_VALUE: Final = re.compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
BEARER_VALUE: Final = re.compile(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{12,}")
PRIVATE_KEY: Final = re.compile(r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----")
SECRET_ASSIGNMENT: Final = re.compile(
    r"(?i)(?:access[_-]?token|refresh[_-]?token|client[_-]?secret|password|recovery[_-]?key|private[_-]?key)"
    r"\s*[\"']?\s*[:=]\s*[\"']?[^\s,}\"']{4,}"
)
URL_USERINFO: Final = re.compile(r"https?://[^\s/:@]+:[^\s/@]+@")
OUTPUT_CHANNELS: Final = frozenset({"audit", "crash", "error", "log", "metric"})


@dataclass(frozen=True)
class SensitiveOutputViolation:
    channel: str
    rule: str


def scan_text(
    channel: str,
    text: str,
    *,
    known_secrets: Sequence[str] = (),
) -> tuple[SensitiveOutputViolation, ...]:
    """返回稳定规则名，不把命中的敏感原文复制进报告。"""
    if channel not in OUTPUT_CHANNELS:
        raise ValueError(f"未知输出通道：{channel}")
    violations: list[SensitiveOutputViolation] = []
    patterns = (
        ("jwt", JWT_VALUE),
        ("bearer", BEARER_VALUE),
        ("private-key", PRIVATE_KEY),
        ("secret-assignment", SECRET_ASSIGNMENT),
        ("url-userinfo", URL_USERINFO),
    )
    for rule, pattern in patterns:
        if pattern.search(text) is not None:
            violations.append(SensitiveOutputViolation(channel, rule))
    if any(secret in text for secret in known_secrets if len(secret) >= 4):
        violations.append(SensitiveOutputViolation(channel, "known-secret"))
    return tuple(violations)


def scan_files(
    files: Iterable[tuple[str, Path]],
    *,
    known_secrets: Sequence[str] = (),
) -> tuple[SensitiveOutputViolation, ...]:
    violations: list[SensitiveOutputViolation] = []
    for channel, path in files:
        if not path.is_file():
            raise FileNotFoundError(path)
        violations.extend(
            scan_text(
                channel,
                path.read_text(encoding="utf-8", errors="replace"),
                known_secrets=known_secrets,
            )
        )
    return tuple(violations)
