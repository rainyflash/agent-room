use std::collections::BTreeMap;

use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    presence::{ProjectedAgentPresence, ProjectedAgentPresenceFields},
    presence_roster::{paginate_roster, project_roster, project_roster_with_presence},
};
use agent_room_domain::{
    agent_lifecycle::{
        AgentArchiveReason, AgentConnection, AgentReception, AgentRosterPolicy,
        MatrixPresenceObservation, MatrixPresenceState,
    },
    agent_status::AgentWorkStatus,
    ids::{AgentId, AgentInstanceId},
    time::UtcMillis,
};
use uuid::Uuid;

fn presence(
    agent: u32,
    instance: u32,
    status: AgentWorkStatus,
    seen: i64,
    expires: i64,
) -> ProjectedAgentPresence {
    record(agent, instance, status, seen, expires, false)
}

/// 名片：工作状态写 `idle`，租约是名义上的 5 分钟。
fn card(agent: u32, published: i64) -> ProjectedAgentPresence {
    record(
        agent,
        agent,
        AgentWorkStatus::Idle,
        published,
        published + 300_000,
        true,
    )
}

fn record(
    agent: u32,
    instance: u32,
    status: AgentWorkStatus,
    seen: i64,
    expires: i64,
    card: bool,
) -> ProjectedAgentPresence {
    ProjectedAgentPresence::from_verified_fields(ProjectedAgentPresenceFields {
        event_id: MatrixEventId::new(format!("$event{agent}-{instance}:room.test")).unwrap(),
        room_id: MatrixRoomId::new("!room:room.test").unwrap(),
        identity: BridgeAgentIdentity::new(
            AgentId::from_uuid(
                Uuid::parse_str(&format!("01990d9e-8400-7000-8000-{agent:012}")).unwrap(),
            ),
            format!("Agent {agent}"),
            format!("@agent{agent}:room.test"),
            AgentInstanceId::from_uuid(
                Uuid::parse_str(&format!("01990d9e-8500-7000-8000-{instance:012}")).unwrap(),
            ),
        )
        .unwrap(),
        status,
        published_at: UtcMillis::new(seen).unwrap(),
        observed_at: UtcMillis::new(seen).unwrap(),
        lease_expires_at: UtcMillis::new(expires).unwrap(),
        origin_server_timestamp: u64::try_from(seen).unwrap(),
        last_polled_at: Some(UtcMillis::new(seen).unwrap()),
        listening_until: None,
        reception_known: true,
        card,
    })
}

fn observed(
    state: MatrixPresenceState,
    offline_seen_at: Option<i64>,
    last_active_at: Option<i64>,
) -> MatrixPresenceObservation {
    MatrixPresenceObservation {
        state,
        offline_seen_at,
        last_active_at,
    }
}

fn project_cards(
    records: &[ProjectedAgentPresence],
    now: i64,
    presence: &BTreeMap<String, MatrixPresenceObservation>,
) -> Vec<agent_room_bridge_core::presence::PresenceObservation> {
    project_roster_with_presence(
        records,
        UtcMillis::new(now).unwrap(),
        AgentRosterPolicy::default(),
        |user_id| presence.get(user_id.as_str()).copied(),
    )
}

#[test]
fn cards_follow_the_matrix_presence_of_their_user() {
    let now = 10 * 86_400_000;
    let records = [
        card(1, 1_000),
        card(2, 1_000),
        card(3, 1_000),
        card(4, 1_000),
    ];
    let presence = BTreeMap::from([
        (
            "@agent1:room.test".to_owned(),
            observed(MatrixPresenceState::Online, None, Some(now - 5_000)),
        ),
        (
            "@agent2:room.test".to_owned(),
            observed(MatrixPresenceState::Unavailable, None, None),
        ),
        (
            "@agent3:room.test".to_owned(),
            observed(MatrixPresenceState::Offline, Some(now - 60_000), None),
        ),
    ]);
    let entries = project_cards(&records, now, &presence);
    let states = entries
        .iter()
        .map(|entry| {
            (
                entry.lifecycle.connection,
                entry.lifecycle.reception,
                entry.lifecycle.offline_since,
                entry.lifecycle.archive_reason,
                entry.status(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            (
                AgentConnection::Online,
                AgentReception::Waiting,
                None,
                None,
                AgentWorkStatus::Idle,
            ),
            (
                AgentConnection::Online,
                AgentReception::OnResume,
                None,
                None,
                AgentWorkStatus::Idle,
            ),
            (
                AgentConnection::Offline,
                AgentReception::Unavailable,
                Some(now - 60_000),
                None,
                AgentWorkStatus::Offline,
            ),
            // 一直没拿到在线状态：离线，从名片的时间算，早就收进以前来过的。
            (
                AgentConnection::Offline,
                AgentReception::Unavailable,
                Some(1_000),
                Some(AgentArchiveReason::Expired),
                AgentWorkStatus::Offline,
            ),
        ]
    );
    assert_eq!(
        entries[0].last_active_at,
        now - 5_000,
        "上次活动用在线状态里的"
    );
    assert_eq!(entries[1].last_active_at, 1_000, "没有就用名片的时间");

    let unknown = project_roster(
        &records,
        UtcMillis::new(now).unwrap(),
        AgentRosterPolicy::default(),
    );
    assert!(
        unknown
            .iter()
            .all(|entry| entry.lifecycle.connection == AgentConnection::Offline),
        "拿不到在线状态的地方，名片都按离线算"
    );
}

#[test]
fn recent_offline_capacity_ranks_cards_by_when_they_went_offline() {
    let now = 2_000_000;
    // 名片很早写的，但最近才离线。
    let mut records = (1..=100)
        .map(|id| card(id, i64::from(id)))
        .collect::<Vec<_>>();
    let mut presence = (1..=100)
        .map(|id| {
            (
                format!("@agent{id}:room.test"),
                observed(
                    MatrixPresenceState::Offline,
                    Some(1_000_000 + i64::from(id)),
                    None,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    // 名片最新，却最早离线：最近离线的名额按离线的那一刻排，它排不上。
    records.push(card(101, 500_000));
    presence.insert(
        "@agent101:room.test".to_owned(),
        observed(MatrixPresenceState::Offline, Some(600_000), None),
    );
    let entries = project_cards(&records, now, &presence);
    let archived = entries
        .iter()
        .filter(|entry| entry.lifecycle.archive_reason == Some(AgentArchiveReason::Capacity))
        .map(|entry| entry.presence().identity().display_name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(archived, ["Agent 101"]);
}

#[test]
fn older_offline_identities_cannot_push_online_agents_off_the_page() {
    let mut records = (1..=1000)
        .map(|id| presence(id, id, AgentWorkStatus::Offline, i64::from(id), 10_000))
        .collect::<Vec<_>>();
    records.push(presence(1001, 1001, AgentWorkStatus::Completed, 0, 200_000));
    let entries = project_roster(
        &records,
        UtcMillis::new(100_000).unwrap(),
        AgentRosterPolicy::default(),
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.lifecycle.archive_reason == Some(AgentArchiveReason::Capacity))
            .count(),
        900
    );
    let first = paginate_roster(entries.clone(), false, None, 100).unwrap();
    let second = paginate_roster(entries.clone(), false, first.next_cursor, 100).unwrap();
    assert_eq!(first.entries.len(), 100);
    assert_eq!(second.entries.len(), 1);
    assert_eq!(
        second.entries[0].lifecycle.connection,
        AgentConnection::Online
    );
    assert!(second.next_cursor.is_none());
    assert_eq!(
        paginate_roster(entries, true, None, 100).unwrap().entries[0]
            .lifecycle
            .archive_reason,
        Some(AgentArchiveReason::Capacity)
    );
}

#[test]
fn one_identity_has_one_state_across_sessions_and_returns_from_archive() {
    let records = [
        presence(1, 1, AgentWorkStatus::Blocked, 0, 10_000),
        presence(1, 2, AgentWorkStatus::Completed, 1000, 20_000),
    ];
    let now = 8 * 86_400_000;
    let archived = project_roster(
        &records,
        UtcMillis::new(now).unwrap(),
        AgentRosterPolicy::default(),
    );
    assert_eq!(archived.len(), 1);
    assert_eq!(
        archived[0].lifecycle.archive_reason,
        Some(AgentArchiveReason::Expired)
    );
    assert_eq!(archived[0].presence().status(), AgentWorkStatus::Completed);
    let mut returned = records.to_vec();
    returned.push(presence(1, 3, AgentWorkStatus::Idle, now, now + 60_000));
    let active = project_roster(
        &returned,
        UtcMillis::new(now).unwrap(),
        AgentRosterPolicy::default(),
    );
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].lifecycle.connection, AgentConnection::Online);
    assert_eq!(active[0].lifecycle.reception, AgentReception::OnResume);
    assert!(active[0].lifecycle.archive_reason.is_none());
}
