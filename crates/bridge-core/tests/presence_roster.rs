use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    presence::{ProjectedAgentPresence, ProjectedAgentPresenceFields},
    presence_roster::{paginate_roster, project_roster},
};
use agent_room_domain::{
    agent_lifecycle::{AgentArchiveReason, AgentConnection, AgentReception, AgentRosterPolicy},
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
    })
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
