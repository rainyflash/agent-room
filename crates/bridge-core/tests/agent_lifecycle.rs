use agent_room_domain::{
    agent_lifecycle::{
        AgentPresenceEvidence, DEFAULT_ARCHIVE_AFTER_DAYS, RECENT_OFFLINE_LIMIT,
        RECEPTION_FRESHNESS_MS, RECONNECT_GRACE_MS,
    },
    agent_status::AgentWorkStatus,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    policy: Policy,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Policy {
    archive_after_days: u16,
    recent_offline_limit: usize,
    reconnect_grace_ms: i64,
    reception_freshness_ms: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    #[serde(default)]
    legacy: bool,
    name: String,
    now: i64,
    status: String,
    last_active: i64,
    expires: i64,
    polled: Option<i64>,
    listening_until: Option<i64>,
    connection: String,
    reception: String,
    offline_since: Option<i64>,
    archived: bool,
}

#[test]
fn rust_and_web_share_the_same_state_boundaries() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../packages/protocol/fixtures/agent-lifecycle.json"
    ))
    .expect("valid cross-client fixture");
    assert_eq!(
        fixture.policy.archive_after_days,
        DEFAULT_ARCHIVE_AFTER_DAYS
    );
    assert_eq!(fixture.policy.recent_offline_limit, RECENT_OFFLINE_LIMIT);
    assert_eq!(fixture.policy.reconnect_grace_ms, RECONNECT_GRACE_MS);
    assert_eq!(
        fixture.policy.reception_freshness_ms,
        RECEPTION_FRESHNESS_MS
    );
    for case in fixture.cases {
        let status = match case.status.as_str() {
            "idle" => AgentWorkStatus::Idle,
            "working" => AgentWorkStatus::Working,
            "completed" => AgentWorkStatus::Completed,
            "offline" => AgentWorkStatus::Offline,
            _ => panic!("unexpected status in fixture"),
        };
        let result = AgentPresenceEvidence {
            reported_status: status,
            lease_expires_at: case.expires,
            last_active_at: case.last_active,
            last_polled_at: case.polled,
            listening_until: case.listening_until,
            reception_known: !case.legacy,
        }
        .lifecycle(case.now, DEFAULT_ARCHIVE_AFTER_DAYS);
        assert_eq!(result.connection.as_str(), case.connection, "{}", case.name);
        assert_eq!(result.reception.as_str(), case.reception, "{}", case.name);
        assert_eq!(result.offline_since, case.offline_since, "{}", case.name);
        assert_eq!(
            result.archive_reason.is_some(),
            case.archived,
            "{}",
            case.name
        );
    }
}
