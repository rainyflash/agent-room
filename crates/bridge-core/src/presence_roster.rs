use std::collections::BTreeMap;

use agent_room_application::ports::MatrixUserId;
use agent_room_domain::{
    agent_lifecycle::{
        AgentArchiveReason, AgentConnection, AgentPresenceEvidence, AgentRosterPolicy,
        MatrixPresenceObservation, RECENT_OFFLINE_LIMIT,
    },
    agent_status::AgentWorkStatus,
    ids::AgentId,
    time::UtcMillis,
};

use crate::presence::{PresenceObservation, ProjectedAgentPresence};

pub struct PresenceRosterPage {
    pub entries: Vec<PresenceObservation>,
    pub next_cursor: Option<AgentId>,
}

/// Filter archived identities before paging; the archive classification itself is room-wide.
/// # Errors
/// Rejects empty and oversized pages.
pub fn paginate_roster(
    mut entries: Vec<PresenceObservation>,
    include_archived: bool,
    after: Option<AgentId>,
    limit: u16,
) -> Result<PresenceRosterPage, crate::presence::PresenceQueryError> {
    if limit == 0 || usize::from(limit) > agent_room_domain::agent_lifecycle::PRESENCE_PAGE_SIZE {
        return Err(crate::presence::PresenceQueryError::InvalidPageSize);
    }
    entries.retain(|entry| {
        (include_archived || entry.lifecycle.archive_reason.is_none())
            && after.is_none_or(|id| entry.presence().identity().agent_id() > id)
    });
    entries.sort_by_key(|entry| entry.presence().identity().agent_id());
    let more = entries.len() > usize::from(limit);
    entries.truncate(usize::from(limit));
    let next_cursor = if more {
        entries
            .last()
            .map(|entry| entry.presence().identity().agent_id())
    } else {
        None
    };
    Ok(PresenceRosterPage {
        entries,
        next_cursor,
    })
}

/// Collapse sessions into stable identities before applying the room-wide archive rule.
/// 写名片的 Agent 在这里拿不到 Matrix 在线状态，按离线算。
pub fn project_roster<'a>(
    presences: impl IntoIterator<Item = &'a ProjectedAgentPresence>,
    now: UtcMillis,
    policy: AgentRosterPolicy,
) -> Vec<PresenceObservation> {
    project_roster_with_presence(presences, now, policy, |_| None)
}

/// 同 [`project_roster`]，写名片的 Agent 按 `matrix_presence` 给的在线状态判断。
pub fn project_roster_with_presence<'a>(
    presences: impl IntoIterator<Item = &'a ProjectedAgentPresence>,
    now: UtcMillis,
    policy: AgentRosterPolicy,
    matrix_presence: impl Fn(&MatrixUserId) -> Option<MatrixPresenceObservation>,
) -> Vec<PresenceObservation> {
    let mut groups: BTreeMap<AgentId, Vec<&ProjectedAgentPresence>> = BTreeMap::new();
    for presence in presences {
        groups
            .entry(presence.identity().agent_id())
            .or_default()
            .push(presence);
    }
    let mut entries = groups
        .into_values()
        .map(|instances| {
            let instances = instances
                .into_iter()
                .map(|presence| {
                    let observed = matrix_presence(presence.identity().matrix_user_id());
                    (presence, observed)
                })
                .collect();
            project_agent(instances, now, policy)
        })
        .collect::<Vec<_>>();
    archive_beyond_capacity(&mut entries);
    entries
}

type ObservedInstance<'a> = (
    &'a ProjectedAgentPresence,
    Option<MatrixPresenceObservation>,
);

fn project_agent(
    mut instances: Vec<ObservedInstance<'_>>,
    now: UtcMillis,
    policy: AgentRosterPolicy,
) -> PresenceObservation {
    let days = policy.archive_after_days();
    let online = |(instance, observed): &&ObservedInstance<'_>| {
        instance
            .evidence_with(*observed)
            .lifecycle(now.value(), days)
            .connection
            == AgentConnection::Online
    };
    instances.sort_by_key(|(presence, observed)| {
        std::cmp::Reverse(priority(presence, presence.evidence_with(*observed), now))
    });
    let (primary, primary_observed) = instances[0];
    let mut evidence = primary.evidence_with(primary_observed);
    evidence.last_active_at = instances
        .iter()
        .map(|(instance, _)| instance.published_at().value())
        .max()
        .unwrap_or(evidence.last_active_at);
    evidence.last_polled_at = instances
        .iter()
        .filter(online)
        .filter_map(|(instance, _)| instance.last_polled_at().map(UtcMillis::value))
        .max();
    evidence.listening_until = instances
        .iter()
        .filter(online)
        .filter_map(|(instance, _)| instance.listening_until().map(UtcMillis::value))
        .max();
    evidence.reception_known = instances
        .iter()
        .filter(online)
        .any(|(instance, _)| instance.evidence().reception_known);
    let lifecycle = evidence.lifecycle(now.value(), days);
    let status = if lifecycle.connection == AgentConnection::Online {
        primary.status()
    } else {
        AgentWorkStatus::Offline
    };
    let mut entry = PresenceObservation::new(primary.clone(), status, now);
    entry.lifecycle = lifecycle;
    // 名片的时间是进房间那一刻；在线状态里有更晚的“上次活动”就用它。
    entry.last_active_at = primary_observed
        .filter(|_| primary.is_card())
        .and_then(|observed| observed.last_active_at)
        .map_or(evidence.last_active_at, |at| {
            at.max(evidence.last_active_at)
        });
    entry.last_polled_at = evidence.last_polled_at;
    entry.listening_until = evidence.listening_until;
    entry.archive_after_days = days;
    entry
}

/// 最近离线的只留 100 个，其余收进“以前来过的”。按最后一次见到它排：名片的时间是进房间
/// 那一刻，不是最后一次见到，所以名片按离线的那一刻排。
fn archive_beyond_capacity(entries: &mut [PresenceObservation]) {
    let mut offline = entries
        .iter_mut()
        .filter(|entry| {
            entry.lifecycle.connection == AgentConnection::Offline
                && entry.lifecycle.archive_reason.is_none()
        })
        .collect::<Vec<_>>();
    offline.sort_by_key(|entry| {
        let last_seen = if entry.presence().is_card() {
            entry
                .lifecycle
                .offline_since
                .unwrap_or(entry.last_active_at)
        } else {
            entry.last_active_at
        };
        (
            std::cmp::Reverse(last_seen),
            entry.presence().identity().agent_id(),
        )
    });
    for entry in offline.into_iter().skip(RECENT_OFFLINE_LIMIT) {
        entry.lifecycle.archive_reason = Some(AgentArchiveReason::Capacity);
    }
}

fn priority(
    presence: &ProjectedAgentPresence,
    evidence: AgentPresenceEvidence,
    now: UtcMillis,
) -> (u8, u8, i64, String) {
    let connection = evidence.lifecycle(now.value(), 7).connection;
    let tier = match connection {
        AgentConnection::Online => 2,
        AgentConnection::Reconnecting => 1,
        AgentConnection::Offline => 0,
    };
    let work = if connection == AgentConnection::Online {
        match presence.status() {
            AgentWorkStatus::Blocked => 5,
            AgentWorkStatus::WaitingInput => 4,
            AgentWorkStatus::Working => 3,
            AgentWorkStatus::Completed => 2,
            AgentWorkStatus::Idle => 1,
            AgentWorkStatus::Offline => 0,
        }
    } else {
        0
    };
    (
        tier,
        work,
        presence.published_at().value(),
        presence.identity().agent_instance_id().to_string(),
    )
}
