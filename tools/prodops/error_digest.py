"""每小时只读汇总一次生产上的错（`production.py error-digest`）。

设计在 `specs/error-triage/design.md`。这里只放纯逻辑：把各服务的日志行认成“哪一种错”（指纹），打码，
去掉噪音表里写着正常的，数次数。读日志在 `runtime.py`，往 GitHub 发在后面一步。

打码是这份文件最要紧的事：汇总会离开服务器（进私有仓库的 issue），里面不能有邮箱、IP、账户和房间的 ID、
名字、令牌和各种一次性参数。`tools/tests/test_error_digest.py` 卡着。
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, datetime
import hashlib
import hmac
import ipaddress
import json
from pathlib import Path
import re
import secrets
import tomllib
from typing import Iterable

SERVICES = ("control-plane", "identity", "synapse", "gateway")
NOISE_FILE = Path(__file__).with_name("error_signatures.toml")
SEVERITIES = ("p1", "p2", "p3")
MAX_SAMPLE_CHARACTERS = 300
MAX_CORRELATIONS = 3

_EMAIL = re.compile(r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+")
_UUID = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
_IPV4 = re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}\b")
# 先挑出十六进制和冒号连成、至少两个冒号的一段，再交给 ipaddress 认：时间（02:50:29）不是 IPv6，别打掉。
_IPV6 = re.compile(r"(?<![\w:.])[0-9a-fA-F:]*:[0-9a-fA-F:]*:[0-9a-fA-F:]*(?![\w:])")
_MATRIX = re.compile(r"(?<![\w])([@!#$])[\w.=/+%-]+(?::[\w.-]+(?::\d+)?)?")
# 前面是点的不算（`error.code=` 是错误码，要留着）。
_SECRET_FIELD = re.compile(
    r"(?<![\w.])(code|state|session_state|nonce|token|access_token|refresh_token|id_token|key|code_id|token_id"
    r"|tab_id|client_data|execution|session|auth_session_id|username|email|userId|user_id|ipAddress"
    r"|redirect_uri|login_hint)(=\"?)[^\"&\s,]*",
)
# 令牌、设备号、会话号这类：够长、字母和数字都有。`agent_room_control_plane` 这种模块名没有数字，留着。
_OPAQUE = re.compile(r"(?=[A-Za-z0-9_-]*\d)(?=[A-Za-z0-9_-]*[A-Za-z])[A-Za-z0-9_-]{20,}")
_CONTROL = re.compile(r"[^\x20-\x7e -￿]")
_NUMBER = re.compile(r"\b\d+(?:\.\d+)?\b")
_HEX_ADDRESS = re.compile(r"\b0x[0-9a-fA-F]+\b")
_QUOTED = re.compile(r"'[^']{1,80}'")


def sanitize(text: str) -> str:
    """去掉能认出人或能拿去用的东西，只留可打印字符，截短。"""

    text = _CONTROL.sub("", text)
    text = _SECRET_FIELD.sub(lambda match: f"{match.group(1)}{match.group(2)}<x>", text)
    text = _EMAIL.sub("<email>", text)
    text = _UUID.sub("<id>", text)
    text = _IPV4.sub("<ip>", text)
    text = _IPV6.sub(_mask_ipv6, text)
    text = _MATRIX.sub(lambda match: _MATRIX_PLACEHOLDER[match.group(1)], text)
    text = _OPAQUE.sub("<x>", text)
    if len(text) > MAX_SAMPLE_CHARACTERS:
        text = text[: MAX_SAMPLE_CHARACTERS - 1] + "…"
    return text


_MATRIX_PLACEHOLDER = {"@": "<user>", "!": "<room>", "#": "<alias>", "$": "<event>"}


def _mask_ipv6(match: re.Match[str]) -> str:
    try:
        ipaddress.IPv6Address(match.group(0))
    except ValueError:
        return match.group(0)
    return "<ip>"


def route_template(path: str) -> str:
    """接口路径里的 ID 换成占位，同一个接口算一种。"""

    path = path.split("?", 1)[0]
    segments = []
    for segment in path.split("/"):
        lowered = segment.lower()
        if _UUID.fullmatch(segment) or lowered.startswith(("%21", "%40", "%24", "%23")) or (
            segment[:1] in "!@$#" and segment
        ):
            segments.append("{id}")
        elif segment.isdigit():
            segments.append("{n}")
        elif len(segment) >= 16 and re.fullmatch(r"[A-Za-z0-9_.=%-]+", segment):
            segments.append("{x}")
        else:
            segments.append(segment)
    return sanitize("/".join(segments))


def message_template(text: str) -> str:
    """一句日志里会变的部分（数字、地址、引号里的值）换成占位，同一句话算一种。"""

    text = sanitize(text)
    text = _HEX_ADDRESS.sub("0x…", text)
    text = _QUOTED.sub("'…'", text)
    return _NUMBER.sub("N", text)


@dataclass(frozen=True, slots=True)
class Signature:
    """一种错。`summary` 已经打过码、换过占位，可以直接给人看。"""

    service: str
    summary: str
    severity: str

    @property
    def fingerprint(self) -> str:
        digest = hashlib.sha256(f"{self.service}\n{self.summary}".encode("utf-8")).hexdigest()
        return digest[:12]


@dataclass(frozen=True, slots=True)
class Observation:
    """日志里的一行认出来的样子。`account` 只在这一次汇总里用来数人，不出这个进程。"""

    signature: Signature
    timestamp: str
    sample: str
    correlation: str | None = None
    account: str | None = None


@dataclass(slots=True)
class Finding:
    signature: Signature
    count: int = 0
    first_seen: str = ""
    last_seen: str = ""
    sample: str = ""
    correlations: list[str] = field(default_factory=list)
    accounts: set[str] = field(default_factory=set)

    def to_json(self) -> dict[str, object]:
        return {
            "fingerprint": self.signature.fingerprint,
            "service": self.signature.service,
            "severity": self.signature.severity,
            "summary": self.signature.summary,
            "count": self.count,
            "accounts": len(self.accounts),
            "firstSeen": self.first_seen,
            "lastSeen": self.last_seen,
            "sample": self.sample,
            "correlations": list(self.correlations),
        }


@dataclass(frozen=True, slots=True)
class NoiseRule:
    service: str
    pattern: re.Pattern[str]
    reason: str


class NoiseTableError(ValueError):
    pass


def load_noise(path: Path = NOISE_FILE) -> tuple[NoiseRule, ...]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    rules = []
    for index, entry in enumerate(data.get("ignore", [])):
        service, summary, reason = entry.get("service"), entry.get("summary"), entry.get("reason")
        if service not in SERVICES or not isinstance(summary, str) or not isinstance(reason, str):
            raise NoiseTableError(f"噪音表第 {index + 1} 条要有 service、summary 和 reason。")
        if not reason.strip():
            raise NoiseTableError(f"噪音表第 {index + 1} 条没写为什么正常。")
        try:
            pattern = re.compile(summary)
        except re.error as error:
            raise NoiseTableError(f"噪音表第 {index + 1} 条的 summary 不是正则：{error}") from error
        rules.append(NoiseRule(service=service, pattern=pattern, reason=reason.strip()))
    return tuple(rules)


@dataclass(slots=True)
class Digest:
    findings: list[Finding]
    ignored: dict[str, int]
    lines: dict[str, int]

    def to_json(self) -> dict[str, object]:
        return {
            "findings": [finding.to_json() for finding in self.findings],
            "ignored": dict(self.ignored),
            "lines": dict(self.lines),
        }


def summarize(
    logs: dict[str, Iterable[str]],
    noise: tuple[NoiseRule, ...],
) -> Digest:
    """把各服务的日志行汇总成发现的问题，按轻重、次数排好。"""

    salt = secrets.token_bytes(16)
    findings: dict[str, Finding] = {}
    ignored: dict[str, int] = {}
    lines: dict[str, int] = {}
    for service, service_lines in logs.items():
        parse = _PARSERS[service]
        count = 0
        for line in service_lines:
            count += 1
            observation = parse(line)
            if observation is None:
                continue
            rule = _matching_rule(observation.signature, noise)
            if rule is not None:
                ignored[rule.reason] = ignored.get(rule.reason, 0) + 1
                continue
            _record(findings, observation, salt)
        lines[service] = count
    ordered = sorted(
        findings.values(),
        key=lambda finding: (SEVERITIES.index(finding.signature.severity), -finding.count),
    )
    return Digest(findings=ordered, ignored=ignored, lines=lines)


def _matching_rule(signature: Signature, noise: tuple[NoiseRule, ...]) -> NoiseRule | None:
    for rule in noise:
        if rule.service == signature.service and rule.pattern.search(signature.summary):
            return rule
    return None


def _record(findings: dict[str, Finding], observation: Observation, salt: bytes) -> None:
    key = observation.signature.fingerprint
    finding = findings.get(key)
    if finding is None:
        finding = Finding(signature=observation.signature, first_seen=observation.timestamp,
                          sample=observation.sample)
        findings[key] = finding
    finding.count += 1
    if observation.timestamp:
        if not finding.first_seen or observation.timestamp < finding.first_seen:
            finding.first_seen = observation.timestamp
        finding.last_seen = max(finding.last_seen, observation.timestamp)
    if observation.correlation is not None and len(finding.correlations) < MAX_CORRELATIONS:
        if observation.correlation not in finding.correlations:
            finding.correlations.append(observation.correlation)
    if observation.account is not None:
        finding.accounts.add(
            hmac.new(salt, observation.account.encode("utf-8"), hashlib.sha256).hexdigest()[:16]
        )


# 控制面：结构化 JSON。WARN、ERROR 每条都看；“请求处理完成”只看 5xx（4xx 多半是扫描器和没登录的请求）。
def parse_control_plane(line: str) -> Observation | None:
    try:
        entry = json.loads(line)
    except ValueError:
        return None
    if not isinstance(entry, dict):
        return None
    fields = entry.get("fields") if isinstance(entry.get("fields"), dict) else {}
    span = entry.get("span") if isinstance(entry.get("span"), dict) else {}
    timestamp = _iso(str(entry.get("timestamp", "")))
    correlation = fields.get("correlation.id") or span.get("correlation.id")
    correlation = correlation if isinstance(correlation, str) and _UUID.fullmatch(correlation) else None
    method = span.get("http.request.method")
    path = span.get("url.path")
    route = f"{method} {route_template(path)}" if isinstance(method, str) and isinstance(path, str) else None
    level = entry.get("level")
    status = fields.get("http.response.status_code")
    if fields.get("message") == "请求处理完成":
        if not isinstance(status, int) or status < 500 or route is None:
            return None
        signature = Signature("control-plane", f"{route} → {status}", "p1")
        return Observation(signature, timestamp, signature.summary, correlation)
    if level not in ("WARN", "ERROR"):
        return None
    message = message_template(str(fields.get("message", "")))
    details = " ".join(
        f"{name}={message_template(str(fields[name]))}"
        for name in ("error.code", "code", "failure", "operation", "reason")
        if name in fields
    )
    target = str(entry.get("target", ""))
    summary = " ".join(part for part in (level, target, message, details) if part)
    if route is not None:
        summary += f" @ {route}"
    signature = Signature("control-plane", sanitize(summary), "p1" if level == "ERROR" else "p2")
    # 样例只带短的、像代码的字段；名字、正文这类自由文字不带。
    sample_fields = {
        name: value
        for name, value in fields.items()
        if name not in ("message", "correlation.id")
        and not _FREE_TEXT_FIELD.search(name)
        and isinstance(value, (int, float, bool, str))
        and len(str(value)) <= 80
    }
    sample = sanitize(f"{message} {json.dumps(sample_fields, ensure_ascii=False)}")
    return Observation(signature, timestamp, sample, correlation)


_FREE_TEXT_FIELD = re.compile(r"name|display|body|text|content|title|summary|description", re.IGNORECASE)


# 身份服务（Keycloak）：`[org.keycloak.events]` 里 `type="..._ERROR"` 的事件，以及别的 ERROR 行。
_KEYCLOAK_LINE = re.compile(
    r"^(?P<date>\d{4}-\d{2}-\d{2}) (?P<time>\d{2}:\d{2}:\d{2}),\d+\s+(?P<level>WARN|ERROR|FATAL)\s+"
    r"\[(?P<logger>[\w.$]+)\]\s+\([^)]*\)\s*(?P<message>.*)$"
)
_KEYCLOAK_FIELD = re.compile(r'(\w+)="([^"]*)"')
_KEYCLOAK_LOGIN_FAILURES = {"LOGIN_ERROR", "CODE_TO_TOKEN_ERROR", "IDENTITY_PROVIDER_LOGIN_ERROR"}


def parse_identity(line: str) -> Observation | None:
    match = _KEYCLOAK_LINE.match(line)
    if match is None:
        return None
    timestamp = f"{match['date']}T{match['time']}Z"
    message = match["message"]
    if match["logger"] == "org.keycloak.events":
        fields = dict(_KEYCLOAK_FIELD.findall(message))
        event = fields.get("type", "")
        if not event.endswith("_ERROR"):
            return None
        error = fields.get("error", "")
        severity = "p1" if event in _KEYCLOAK_LOGIN_FAILURES else "p2"
        summary = sanitize(f"{event} error={error}")
        detail = " ".join(
            f"{name}={fields[name]}"
            for name in ("clientId", "reason", "action", "response_type", "response_mode")
            if name in fields
        )
        account = fields.get("userId")
        account = account if account and account != "null" else None
        return Observation(
            Signature("identity", summary, severity), timestamp, sanitize(f"{summary} {detail}"),
            account=account,
        )
    if match["level"] == "WARN":
        severity = "p3"
    else:
        severity = "p1"
    summary = f"{match['level']} {match['logger']} {message_template(message)}"
    return Observation(Signature("identity", sanitize(summary), severity), timestamp, sanitize(message))


# Synapse：访问日志只看登录了的人收到的 4xx 和所有 5xx（没登录的 4xx 是扫描器）；别的 WARNING、ERROR 看一句话。
_SYNAPSE_ACCESS = re.compile(
    r"^(?P<date>\d{4}-\d{2}-\d{2}) (?P<time>\d{2}:\d{2}:\d{2}),\d+ \w+ synapse\.access\.http\.\d+ \S+ - \d+ - "
    r"\{(?P<user>[^}]*)\} Processed request: .*? (?P<status>\d{3}) \"(?P<method>[A-Z]+) (?P<path>[^ \"]+)"
)
_SYNAPSE_LINE = re.compile(
    r"^(?P<date>\d{4}-\d{2}-\d{2}) (?P<time>\d{2}:\d{2}:\d{2}),\d+ (?P<level>WARNING|ERROR|CRITICAL) "
    r"(?P<logger>[\w.]+) (?P<message>.*)$"
)


def parse_synapse(line: str) -> Observation | None:
    access = _SYNAPSE_ACCESS.match(line)
    if access is not None:
        status = int(access["status"])
        user = access["user"]
        signed_in = user not in ("", "None", "-")
        if status < 400 or (status < 500 and not signed_in):
            return None
        route = f"{access['method']} {route_template(access['path'])}"
        signature = Signature("synapse", f"{route} → {status}", "p1" if status >= 500 else "p2")
        return Observation(
            signature, f"{access['date']}T{access['time']}Z", signature.summary,
            account=user if signed_in else None,
        )
    match = _SYNAPSE_LINE.match(line)
    if match is None:
        return None
    severity = "p3" if match["level"] == "WARNING" else "p1"
    summary = f"{match['level']} {match['logger']} {message_template(match['message'])}"
    return Observation(
        Signature("synapse", sanitize(summary), severity), f"{match['date']}T{match['time']}Z",
        sanitize(match["message"]),
    )


# 网关（Caddy）：JSON，看 warn 和 error。
def parse_gateway(line: str) -> Observation | None:
    try:
        entry = json.loads(line)
    except ValueError:
        return None
    if not isinstance(entry, dict) or entry.get("level") not in ("warn", "error"):
        return None
    level = entry["level"]
    logger = str(entry.get("logger", ""))
    message = str(entry.get("msg", ""))
    timestamp = entry.get("ts")
    stamp = (
        _from_unix(float(timestamp)) if isinstance(timestamp, (int, float)) else ""
    )
    summary = sanitize(f"{level} {logger} {message_template(message)}")
    return Observation(
        Signature("gateway", summary, "p2" if level == "error" else "p3"), stamp, sanitize(message)
    )


_PARSERS = {
    "control-plane": parse_control_plane,
    "identity": parse_identity,
    "synapse": parse_synapse,
    "gateway": parse_gateway,
}


_DURATION = re.compile(r"(\d+)([mhd])")
_DURATION_UNITS = {"m": 60, "h": 3600, "d": 86400}


def parse_window(since: str, until: str | None, now: datetime) -> tuple[datetime, datetime]:
    """`--since` 可以是 `1h`、`30m`、`2d` 这样往回数，也可以是带时区的时刻；`--until` 不给就是现在。"""

    end = now if until is None else _parse_instant(until)
    match = _DURATION.fullmatch(since)
    start = (
        datetime.fromtimestamp(end.timestamp() - int(match[1]) * _DURATION_UNITS[match[2]], UTC)
        if match
        else _parse_instant(since)
    )
    if start >= end:
        raise ValueError("--since 要早于 --until。")
    return start, end


def _parse_instant(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError(f"认不出这个时刻：{value}（要带时区，比如 2026-10-24T02:00:00Z）") from error
    if parsed.tzinfo is None:
        raise ValueError(f"时刻要带时区：{value}")
    return parsed.astimezone(UTC)


def _iso(value: str) -> str:
    match = re.match(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})", value)
    return f"{match.group(1)}Z" if match else ""


def _from_unix(value: float) -> str:
    return datetime.fromtimestamp(value, UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def report(digest: Digest, *, since: str, until: str, version: str) -> list[str]:
    """给人看的汇总（`--format markdown`），每一行都已经打过码。"""

    lines = [
        f"# 生产错误汇总 {since} – {until}",
        "",
        f"版本 {version}；读了 "
        + "、".join(f"{service} {count} 行" for service, count in digest.lines.items())
        + "。",
        "",
    ]
    if not digest.findings:
        lines.append("没有要报的。")
    for finding in digest.findings:
        signature = finding.signature
        # 认得出账户的日志（身份服务、Synapse）才写涉及几个账户，控制面的日志里没有账户。
        accounts = f"涉及 {len(finding.accounts)} 个账户，" if finding.accounts else ""
        lines.extend(
            [
                f"## [{signature.severity}] {signature.service}：{signature.summary}",
                "",
                f"- 指纹 `{signature.fingerprint}`，{finding.count} 次，{accounts}"
                f"{finding.first_seen} 到 {finding.last_seen}",
                f"- 样例：`{finding.sample.replace('`', 'ˋ')}`",
            ]
        )
        if finding.correlations:
            lines.append("- 关联号：" + "、".join(f"`{value}`" for value in finding.correlations))
        lines.append("")
    if digest.ignored:
        lines.extend(["## 按噪音表没报的", ""])
        for reason, count in sorted(digest.ignored.items(), key=lambda item: -item[1]):
            lines.append(f"- {count} 次：{reason}")
    return lines
