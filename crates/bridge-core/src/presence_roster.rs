use std::collections::BTreeMap;

use agent_room_domain::{
    agent_lifecycle::{
        AgentArchiveReason, AgentConnection, AgentRosterPolicy, RECENT_OFFLINE_LIMIT,
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
pub fn project_roster<'a>(
    presences: impl IntoIterator<Item = &'a ProjectedAgentPresence>,
    now: UtcMillis,
    policy: AgentRosterPolicy,
) -> Vec<PresenceObservation> {
    let mut groups: BTreeMap<AgentId, Vec<&ProjectedAgentPresence>> = BTreeMap::new();
    for presence in presences {
        groups
            .entry(presence.identity().agent_id())
            .or_default()
            .push(presence);
    }
    let mut entries = Vec::with_capacity(groups.len());
    for instances in groups.values_mut() {
        instances.sort_by_key(|presence| std::cmp::Reverse(priority(presence, now)));
        let primary = instances[0];
        let mut evidence = primary.evidence();
        evidence.last_active_at = instances
            .iter()
            .map(|instance| instance.published_at().value())
            .max()
            .unwrap_or(evidence.last_active_at);
        evidence.last_polled_at = instances
            .iter()
            .filter(|instance| {
                instance
                    .evidence()
                    .lifecycle(now.value(), policy.archive_after_days())
                    .connection
                    == AgentConnection::Online
            })
            .filter_map(|instance| instance.last_polled_at().map(UtcMillis::value))
            .max();
        evidence.listening_until = instances
            .iter()
            .filter(|instance| {
                instance
                    .evidence()
                    .lifecycle(now.value(), policy.archive_after_days())
                    .connection
                    == AgentConnection::Online
            })
            .filter_map(|instance| instance.listening_until().map(UtcMillis::value))
            .max();
        evidence.reception_known = instances.iter().any(|instance| {
            instance.evidence().reception_known
                && instance
                    .evidence()
                    .lifecycle(now.value(), policy.archive_after_days())
                    .connection
                    == AgentConnection::Online
        });
        let lifecycle = evidence.lifecycle(now.value(), policy.archive_after_days());
        let status = if lifecycle.connection == AgentConnection::Online {
            primary.status()
        } else {
            AgentWorkStatus::Offline
        };
        let mut entry = PresenceObservation::new(primary.clone(), status, now);
        entry.lifecycle = lifecycle;
        entry.last_active_at = evidence.last_active_at;
        entry.last_polled_at = evidence.last_polled_at;
        entry.listening_until = evidence.listening_until;
        entry.archive_after_days = policy.archive_after_days();
        entries.push(entry);
    }
    let mut offline = entries
        .iter_mut()
        .filter(|entry| {
            entry.lifecycle.connection == AgentConnection::Offline
                && entry.lifecycle.archive_reason.is_none()
        })
        .collect::<Vec<_>>();
    offline.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.last_active_at),
            entry.presence().identity().agent_id(),
        )
    });
    for entry in offline.into_iter().skip(RECENT_OFFLINE_LIMIT) {
        entry.lifecycle.archive_reason = Some(AgentArchiveReason::Capacity);
    }
    entries
}

fn priority(presence: &ProjectedAgentPresence, now: UtcMillis) -> (u8, u8, i64, String) {
    let connection = presence.evidence().lifecycle(now.value(), 7).connection;
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
