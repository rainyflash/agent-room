from __future__ import annotations

from datetime import UTC, datetime
import json
import re
import unittest

from tools.prodops.error_digest import (
    NOISE_FILE,
    SERVICES,
    load_noise,
    parse_control_plane,
    parse_gateway,
    parse_identity,
    parse_synapse,
    parse_window,
    report,
    route_template,
    sanitize,
    summarize,
)

# 下面的日志行照 2026-10-24 生产上的格式写，账户、房间、IP、邮箱、令牌都是编的。
USER_ID = "@user-594e1f403740d561dd0176d9ba58c5de:agentroom.chat"
AGENT_ID = "@_agent_01a165bc4afc7a71a62495e9a454ecf6:agentroom.chat"
ROOM_ID = "!NoFNSoWeMyYPNJjkDK:agentroom.chat"
CATALOG_ID = "01a165bc-3e68-7850-bd56-b141bd0307af"
CORRELATION_ID = "01a165c7-078f-7a30-b008-ac3d293a6a9a"
KEYCLOAK_USER = "9121f385-13da-4f8a-a472-c262a6d9beec"
EMAIL = "someone.real@qq.com"
IP = "203.0.113.42"
IPV6 = "2001:db8:85a3::8a2e:370:7334"

CONTROL_WARN = json.dumps(
    {
        "timestamp": "2026-10-22T22:41:44.851309Z",
        "level": "WARN",
        "fields": {"message": "公开大厅云端入场失败", "correlation.id": CORRELATION_ID, "failure": "Internal"},
        "target": "agent_room_control_plane::features::lobbies",
        "span": {
            "correlation.id": CORRELATION_ID,
            "http.request.method": "POST",
            "otel.kind": "server",
            "url.path": f"/lobbies/{CATALOG_ID}/entry",
            "name": "http.request",
        },
    },
    ensure_ascii=False,
)
CONTROL_500 = json.dumps(
    {
        "timestamp": "2026-10-22T22:41:44.851400Z",
        "level": "INFO",
        "fields": {"message": "请求处理完成", "http.response.status_code": 500, "latency_ms": 3},
        "target": "agent_room_control_plane::correlation",
        "span": {
            "correlation.id": CORRELATION_ID,
            "http.request.method": "POST",
            "url.path": f"/lobbies/{CATALOG_ID}/entry",
        },
    },
    ensure_ascii=False,
)
CONTROL_404 = CONTROL_500.replace('"http.response.status_code": 500', '"http.response.status_code": 404')
CONTROL_CALLBACK = json.dumps(
    {
        "timestamp": "2026-10-23T00:34:52.733452Z",
        "level": "WARN",
        "fields": {
            "message": "浏览器登录回调失败，返回连接页重新开始",
            "correlation.id": "01a1662e-9abc-7fb1-9ee5-b73b0c23f980",
            "error.code": "authentication.invalid_callback_query",
            "display_name": "小明的电脑",
        },
        "target": "agent_room_control_plane::features::authentication",
        "span": {"http.request.method": "GET", "url.path": "/auth/oidc/callback"},
    },
    ensure_ascii=False,
)
CONTROL_JOIN_CODE = json.dumps(
    {
        "timestamp": "2026-10-22T22:30:28.000000Z",
        "level": "WARN",
        "fields": {
            "message": "Agent 口令请求失败",
            "correlation.id": CORRELATION_ID,
            "operation": "private_room.join_code.reveal",
            "failure": "NotFound",
        },
        "target": "agent_room_control_plane::error",
        "span": {"http.request.method": "GET", "url.path": f"/devices/current/private-rooms/{CATALOG_ID}/code"},
    },
    ensure_ascii=False,
)
KEYCLOAK_LOGIN_ERROR = (
    '2026-10-23 00:34:51,492 WARN  [org.keycloak.events] (executor-thread-520) type="LOGIN_ERROR", '
    'realmId="eb721f30-eb9f-46dc-bc4a-68d6e3922966", realmName="agent-room", clientId="agent-room-web", '
    f'userId="null", ipAddress="{IP}", error="invalid_request", reason="Invalid scopes: openidprofile", '
    'response_type="code", redirect_uri="https://app.agentroom.chat/connect/finalize", response_mode="query"'
)
KEYCLOAK_VERIFY = (
    '2026-10-22 22:27:49,860 WARN  [org.keycloak.events] (executor-thread-438) type="VERIFY_EMAIL_ERROR", '
    f'realmId="eb721f30-eb9f-46dc-bc4a-68d6e3922966", realmName="agent-room", clientId="agent-room-web", '
    f'userId="{KEYCLOAK_USER}", ipAddress="{IP}", error="email_already_verified", auth_method="openid-connect", '
    'token_id="b0f24bca-f03c-433a-85f8-dbe62c151f8b", action="verify-email", response_type="code", '
    f'code_id="1a3hp4Sn2T-z9OPX28GG0z53", email="{EMAIL}", username="{EMAIL}", response_mode="query"'
)
KEYCLOAK_EXPIRED = (
    '2026-10-23 00:33:17,795 WARN  [org.keycloak.events] (executor-thread-519) type="EXECUTE_ACTION_TOKEN_ERROR", '
    f'realmId="eb721f30", realmName="agent-room", clientId="null", userId="{KEYCLOAK_USER}", ipAddress="{IP}", '
    'error="expired_code", reason="Token is not active", token_id="b0f24bca-f03c-433a-85f8-dbe62c151f8b", '
    'action="verify-email"'
)
KEYCLOAK_SERVICES = (
    "2026-10-23 00:34:51,490 ERROR [org.keycloak.services] (executor-thread-520) "
    "KC-SERVICES0093: Invalid parameter value for: scope"
)
KEYCLOAK_INFO = (
    '2026-10-23 00:34:51,490 INFO  [org.keycloak.events] (executor-thread-520) type="LOGIN", userId="x"'
)
SYNAPSE_ACCESS_403 = (
    f"2026-10-24 02:50:29,279 INFO synapse.access.http.8008 {IP} - 8008 - {{{USER_ID}}} Processed request: "
    "0.005sec/0.000sec ru=(0.002sec, 0.000sec) db=(0.001sec/0.002sec/2) 77B 403 "
    f'"GET /_matrix/client/v3/presence/{USER_ID}/status HTTP/1.1" "Mozilla/5.0 (Macintosh)" [0 dbevts]'
)
SYNAPSE_ACCESS_ANONYMOUS_404 = (
    f"2026-10-24 02:50:29,279 INFO synapse.access.http.8008 {IP} - 8008 - {{None}} Processed request: "
    '0.001sec/0.000sec ru=(0.000sec, 0.000sec) db=(0.000sec/0.000sec/0) 2B 404 "GET /.env HTTP/1.1" "curl" [0 dbevts]'
)
SYNAPSE_ACCESS_500 = (
    f"2026-10-24 02:51:00,001 INFO synapse.access.http.8008 {IP} - 8008 - {{{AGENT_ID}}} Processed request: "
    "0.005sec/0.000sec ru=(0.002sec, 0.000sec) db=(0.001sec/0.002sec/2) 77B 500 "
    f'"PUT /_matrix/client/v3/rooms/{ROOM_ID}/send/m.room.encrypted/m1729 HTTP/1.1" "matrix-rust-sdk" [0 dbevts]'
)
SYNAPSE_ACCESS_200 = SYNAPSE_ACCESS_403.replace(" 77B 403 ", " 77B 200 ")
SYNAPSE_WARNING = (
    "2026-10-22 22:30:00,123 WARNING synapse.http.server Not sending response to request "
    "<XForwardedForRequest at 0x798dbbeb0cd0 method='GET' uri='/_matrix/client/v3/sync?filter=0&timeout=30000' "
    "clientproto='HTTP/1.1' site='8008'>, already disconnected."
)
SYNAPSE_ERROR = (
    "2026-10-22 22:30:01,000 ERROR synapse.federation.sender Failed to send to example.org: "
    f"room {ROOM_ID} event $AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 for {USER_ID}"
)
GATEWAY_ERROR = json.dumps(
    {
        "level": "error",
        "ts": 1792680855.1037943,
        "logger": "http.log.error",
        "msg": f"dial tcp {IP}:8080: connect: connection refused",
    }
)


class SanitizeTests(unittest.TestCase):
    def test_removes_everything_that_identifies_someone(self) -> None:
        text = (
            f"{EMAIL} {IP} {IPV6} {USER_ID} {ROOM_ID} {CATALOG_ID} #alias:agentroom.chat "
            "$AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 code=abc123 state=xyz token_id=\"t1\" "
            'username="bob" email="bob@example.com" userId="u1" ipAddress="198.51.100.1" '
            "access_token=syt_c29tZW9uZQ_AbCdEfGhIjKlMnOp_1a2b3c AR_01a165bc76827cd39686e0320733011e "
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"
        )
        cleaned = sanitize(text)
        for secret in (
            EMAIL, IP, IPV6, "user-594e1f40", "NoFNSoWeMyYPNJjkDK", CATALOG_ID, "alias:agentroom",
            "AbCdEfGhIjKlMnOp", "abc123", "xyz", '"t1"', "bob", '"u1"', "198.51.100.1", "syt_",
            "01a165bc76827cd3", "eyJhbGciOiJIUzI1NiJ9", "dozjgNryP4J3",
        ):
            self.assertNotIn(secret, cleaned)

    def test_keeps_module_names_and_error_codes(self) -> None:
        text = (
            "agent_room_control_plane::features::lobbies error.code=authentication.invalid_callback_query "
            "KC-SERVICES0093 at 02:50:29"
        )
        self.assertEqual(sanitize(text), text)

    def test_drops_control_characters_and_truncates(self) -> None:
        cleaned = sanitize("a\x1b[31mb\r\n" + "x" * 1000)
        self.assertNotIn("\x1b", cleaned)
        self.assertNotIn("\n", cleaned)
        self.assertLessEqual(len(cleaned), 300)

    def test_route_template_merges_ids(self) -> None:
        self.assertEqual(route_template(f"/lobbies/{CATALOG_ID}/entry?x=1"), "/lobbies/{id}/entry")
        self.assertEqual(
            route_template(f"/_matrix/client/v3/rooms/{ROOM_ID}/state/m.room.member/{USER_ID}"),
            "/_matrix/client/v3/rooms/{id}/state/m.room.member/{id}",
        )
        self.assertEqual(
            route_template("/_matrix/client/v3/rooms/%21abc%3Aagentroom.chat/messages"),
            "/_matrix/client/v3/rooms/{id}/messages",
        )
        self.assertEqual(route_template("/agents/42/instances"), "/agents/{n}/instances")


class ParserTests(unittest.TestCase):
    def test_control_plane_warning(self) -> None:
        observation = parse_control_plane(CONTROL_WARN)
        assert observation is not None
        self.assertEqual(
            observation.signature.summary,
            "WARN agent_room_control_plane::features::lobbies 公开大厅云端入场失败 failure=Internal "
            "@ POST /lobbies/{id}/entry",
        )
        self.assertEqual(observation.signature.severity, "p2")
        self.assertEqual(observation.correlation, CORRELATION_ID)
        self.assertEqual(observation.timestamp, "2026-10-22T22:41:44Z")

    def test_control_plane_5xx_but_not_4xx(self) -> None:
        observation = parse_control_plane(CONTROL_500)
        assert observation is not None
        self.assertEqual(observation.signature.summary, "POST /lobbies/{id}/entry → 500")
        self.assertEqual(observation.signature.severity, "p1")
        self.assertIsNone(parse_control_plane(CONTROL_404))

    def test_control_plane_sample_leaves_out_free_text(self) -> None:
        observation = parse_control_plane(CONTROL_CALLBACK)
        assert observation is not None
        self.assertIn("authentication.invalid_callback_query", observation.signature.summary)
        self.assertNotIn("小明", observation.sample)

    def test_control_plane_ignores_other_lines(self) -> None:
        self.assertIsNone(parse_control_plane("not json"))
        self.assertIsNone(parse_control_plane(json.dumps({"level": "INFO", "fields": {"message": "ok"}})))

    def test_identity_events(self) -> None:
        login = parse_identity(KEYCLOAK_LOGIN_ERROR)
        assert login is not None
        self.assertEqual(login.signature.summary, "LOGIN_ERROR error=invalid_request")
        self.assertEqual(login.signature.severity, "p1")
        self.assertIn("Invalid scopes: openidprofile", login.sample)
        self.assertIsNone(login.account)
        verify = parse_identity(KEYCLOAK_VERIFY)
        assert verify is not None
        self.assertEqual(verify.account, KEYCLOAK_USER)
        for secret in (EMAIL, IP, KEYCLOAK_USER, "1a3hp4Sn2T"):
            self.assertNotIn(secret, verify.sample)
        self.assertIsNone(parse_identity(KEYCLOAK_INFO))

    def test_identity_event_names_with_digits_are_kept(self) -> None:
        line = KEYCLOAK_LOGIN_ERROR.replace("LOGIN_ERROR", "OAUTH2_DEVICE_CODE_TO_TOKEN_ERROR")
        observation = parse_identity(line)
        assert observation is not None
        self.assertTrue(observation.signature.summary.startswith("OAUTH2_DEVICE_CODE_TO_TOKEN_ERROR error="))
        self.assertNotIn("<x>", observation.signature.summary)

    def test_identity_error_lines(self) -> None:
        observation = parse_identity(KEYCLOAK_SERVICES)
        assert observation is not None
        self.assertEqual(
            observation.signature.summary,
            "ERROR org.keycloak.services KC-SERVICES0093: Invalid parameter value for: scope",
        )
        self.assertEqual(observation.signature.severity, "p1")

    def test_synapse_access(self) -> None:
        forbidden = parse_synapse(SYNAPSE_ACCESS_403)
        assert forbidden is not None
        self.assertEqual(
            forbidden.signature.summary, "GET /_matrix/client/v3/presence/{id}/status → 403"
        )
        self.assertEqual(forbidden.account, USER_ID)
        self.assertNotIn("user-594e", forbidden.sample)
        failed = parse_synapse(SYNAPSE_ACCESS_500)
        assert failed is not None
        self.assertEqual(failed.signature.severity, "p1")
        self.assertIsNone(parse_synapse(SYNAPSE_ACCESS_ANONYMOUS_404))
        self.assertIsNone(parse_synapse(SYNAPSE_ACCESS_200))

    def test_synapse_log_lines_merge_by_template(self) -> None:
        first = parse_synapse(SYNAPSE_WARNING)
        second = parse_synapse(SYNAPSE_WARNING.replace("0x798dbbeb0cd0", "0x798db5506fd0"))
        assert first is not None and second is not None
        self.assertEqual(first.signature.fingerprint, second.signature.fingerprint)
        self.assertEqual(first.signature.severity, "p3")
        error = parse_synapse(SYNAPSE_ERROR)
        assert error is not None
        self.assertEqual(error.signature.severity, "p1")
        for secret in ("NoFNSoWeMyYPNJjkDK", "AbCdEfGhIjKl", "user-594e"):
            self.assertNotIn(secret, error.signature.summary)
            self.assertNotIn(secret, error.sample)

    def test_gateway(self) -> None:
        observation = parse_gateway(GATEWAY_ERROR)
        assert observation is not None
        self.assertEqual(observation.signature.severity, "p2")
        self.assertNotIn(IP, observation.sample)
        self.assertEqual(observation.timestamp[:4], "2026")
        self.assertIsNone(parse_gateway(json.dumps({"level": "info", "msg": "ok"})))


class NoiseTableTests(unittest.TestCase):
    def test_every_rule_has_a_reason_and_compiles(self) -> None:
        rules = load_noise(NOISE_FILE)
        self.assertGreater(len(rules), 0)
        for rule in rules:
            self.assertIn(rule.service, SERVICES)
            self.assertTrue(rule.reason)

    def test_known_noise_is_not_reported_but_real_problems_are(self) -> None:
        presence_429 = SYNAPSE_ACCESS_403.replace(" 77B 403 ", " 77B 429 ").replace('"GET ', '"PUT ')
        account_data_404 = SYNAPSE_ACCESS_403.replace(" 77B 403 ", " 77B 404 ").replace(
            f"/presence/{USER_ID}/status", f"/user/{USER_ID}/account_data/m.secret_storage.default_key"
        )
        digest = summarize(
            {
                "control-plane": [CONTROL_WARN, CONTROL_500, CONTROL_404, CONTROL_CALLBACK, CONTROL_JOIN_CODE],
                "identity": [KEYCLOAK_LOGIN_ERROR, KEYCLOAK_VERIFY, KEYCLOAK_VERIFY, KEYCLOAK_SERVICES],
                "synapse": [SYNAPSE_WARNING, presence_429, account_data_404, SYNAPSE_ACCESS_403],
                "gateway": [],
            },
            load_noise(NOISE_FILE),
        )
        summaries = [finding.signature.summary for finding in digest.findings]
        self.assertIn("POST /lobbies/{id}/entry → 500", summaries)
        self.assertIn("LOGIN_ERROR error=invalid_request", summaries)
        self.assertIn("GET /_matrix/client/v3/presence/{id}/status → 403", summaries)
        self.assertNotIn("VERIFY_EMAIL_ERROR error=email_already_verified", summaries)
        self.assertFalse(any("presence/{id}/status → 429" in summary for summary in summaries))
        self.assertFalse(any("account_data" in summary for summary in summaries))
        self.assertFalse(any("Not sending response" in summary for summary in summaries))
        self.assertFalse(any("join_code" in summary for summary in summaries))
        self.assertEqual(sum(digest.ignored.values()), 6)
        self.assertEqual(digest.findings[0].signature.severity, "p1")

    def test_identity_events_seen_in_production_are_noise(self) -> None:
        def event(kind: str, error: str) -> str:
            return KEYCLOAK_LOGIN_ERROR.replace("LOGIN_ERROR", kind).replace("invalid_request", error)

        digest = summarize(
            {
                "identity": [
                    event("CUSTOM_REQUIRED_ACTION_ERROR", "already_logged_in"),
                    event("CUSTOM_REQUIRED_ACTION_ERROR", "invalid_code"),
                    event("REGISTER_ERROR", "invalid_registration"),
                    event("REGISTER_ERROR", "expired_code"),
                    event("OAUTH2_DEVICE_VERIFY_USER_CODE_ERROR", "invalid_oauth2_user_code"),
                    event("LOGIN_ERROR", "user_not_found"),
                    event("LOGIN_ERROR", "expired_code"),
                    KEYCLOAK_LOGIN_ERROR,
                ]
            },
            load_noise(NOISE_FILE),
        )
        summaries = [finding.signature.summary for finding in digest.findings]
        self.assertEqual(summaries, ["LOGIN_ERROR error=invalid_request"])
        self.assertEqual(sum(digest.ignored.values()), 7)

    def test_network_agent_naming_a_room_that_is_not_there_is_noise(self) -> None:
        def failure(kind: str) -> str:
            return json.dumps(
                {
                    "timestamp": "2026-10-26T04:14:30.623000Z",
                    "level": "WARN",
                    "fields": {"message": "网络 Agent 请求失败", "correlation.id": CORRELATION_ID, "failure": kind},
                    "target": "agent_room_control_plane::error",
                    "span": {"http.request.method": "POST", "url.path": "/mcp"},
                },
                ensure_ascii=False,
            )

        digest = summarize(
            {"control-plane": [failure("RoomNotFound"), failure("Internal")]}, load_noise(NOISE_FILE)
        )
        summaries = [finding.signature.summary for finding in digest.findings]
        self.assertEqual(
            summaries, ["WARN agent_room_control_plane::error 网络 Agent 请求失败 failure=Internal @ POST /mcp"]
        )
        self.assertEqual(sum(digest.ignored.values()), 1)


class SummaryTests(unittest.TestCase):
    def test_counts_and_accounts_without_keeping_who(self) -> None:
        other_user = SYNAPSE_ACCESS_403.replace("user-594e1f40", "user-aaaa1111")
        digest = summarize(
            {"synapse": [SYNAPSE_ACCESS_403, SYNAPSE_ACCESS_403, other_user]}, load_noise(NOISE_FILE)
        )
        [finding] = digest.findings
        self.assertEqual(finding.count, 3)
        self.assertEqual(len(finding.accounts), 2)
        payload = json.dumps(digest.to_json(), ensure_ascii=False)
        self.assertEqual(json.loads(payload)["findings"][0]["accounts"], 2)
        self.assertNotIn("user-", payload)

    def test_report_has_no_identifying_data(self) -> None:
        lines = [CONTROL_WARN, CONTROL_500, CONTROL_CALLBACK]
        digest = summarize(
            {
                "control-plane": lines,
                "identity": [KEYCLOAK_LOGIN_ERROR, KEYCLOAK_EXPIRED, KEYCLOAK_SERVICES],
                "synapse": [SYNAPSE_ACCESS_403, SYNAPSE_ACCESS_500, SYNAPSE_ERROR],
                "gateway": [GATEWAY_ERROR],
            },
            (),
        )
        text = "\n".join(
            report(digest, since="2026-10-22T22:00:00Z", until="2026-10-22T23:00:00Z", version="0.1.0-alpha.80")
        ) + json.dumps(digest.to_json(), ensure_ascii=False)
        for secret in (
            EMAIL, IP, "user-594e", "_agent_01a165bc", "NoFNSoWeMyYPNJjkDK", CATALOG_ID, KEYCLOAK_USER,
            "b0f24bca", "小明",
        ):
            self.assertNotIn(secret, text)
        self.assertIn(CORRELATION_ID, text)
        self.assertIsNone(re.search(r"\b\d{1,3}(?:\.\d{1,3}){3}\b", text.replace("0.1.0", "")))

    def test_window(self) -> None:
        now = datetime(2026, 10, 24, 3, 0, tzinfo=UTC)
        self.assertEqual(
            parse_window("1h", None, now), (datetime(2026, 10, 24, 2, 0, tzinfo=UTC), now)
        )
        self.assertEqual(
            parse_window("2026-10-24T01:30:00Z", "2026-10-24T02:00:00+00:00", now),
            (datetime(2026, 10, 24, 1, 30, tzinfo=UTC), datetime(2026, 10, 24, 2, 0, tzinfo=UTC)),
        )
        with self.assertRaises(ValueError):
            parse_window("2026-10-24T01:30:00", None, now)
        with self.assertRaises(ValueError):
            parse_window("2026-10-24T04:00:00Z", None, now)


if __name__ == "__main__":
    unittest.main()
