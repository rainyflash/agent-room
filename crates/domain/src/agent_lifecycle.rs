//! Connection evidence and recoverable roster archival are independent of task completion.
use crate::agent_status::AgentWorkStatus;

pub const RECONNECT_GRACE_MS: i64 = 30_000;
pub const RECEPTION_FRESHNESS_MS: i64 = 15_000;
pub const DEFAULT_ARCHIVE_AFTER_DAYS: u16 = 7;
pub const RECENT_OFFLINE_LIMIT: usize = 100;
pub const PRESENCE_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentConnection {
    Online,
    Reconnecting,
    Offline,
}
impl AgentConnection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Reconnecting => "reconnecting",
            Self::Offline => "offline",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentReception {
    Waiting,
    OnResume,
    Unknown,
    Unavailable,
}
impl AgentReception {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::OnResume => "on_resume",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentArchiveReason {
    Expired,
    Capacity,
}
impl AgentArchiveReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Capacity => "capacity",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentPresenceEvidence {
    pub reported_status: AgentWorkStatus,
    pub lease_expires_at: i64,
    pub last_active_at: i64,
    pub last_polled_at: Option<i64>,
    pub listening_until: Option<i64>,
    pub reception_known: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLifecycle {
    pub connection: AgentConnection,
    pub reception: AgentReception,
    pub offline_since: Option<i64>,
    pub archive_reason: Option<AgentArchiveReason>,
}
impl AgentPresenceEvidence {
    pub fn lifecycle(self, now: i64, archive_after_days: u16) -> AgentLifecycle {
        let connection = if self.reported_status == AgentWorkStatus::Offline {
            AgentConnection::Offline
        } else if now < self.lease_expires_at {
            AgentConnection::Online
        } else if now < self.lease_expires_at.saturating_add(RECONNECT_GRACE_MS) {
            AgentConnection::Reconnecting
        } else {
            AgentConnection::Offline
        };
        let offline_since = (connection == AgentConnection::Offline).then_some(
            if self.reported_status == AgentWorkStatus::Offline {
                self.last_active_at
            } else {
                self.lease_expires_at.saturating_add(RECONNECT_GRACE_MS)
            },
        );
        let reception = if connection != AgentConnection::Online {
            AgentReception::Unavailable
        } else if self.listening_until.is_some_and(|until| now < until) {
            AgentReception::Waiting
        } else if self.reception_known {
            AgentReception::OnResume
        } else {
            AgentReception::Unknown
        };
        AgentLifecycle {
            connection,
            reception,
            offline_since,
            archive_reason: offline_since
                .filter(|since| now - since >= i64::from(archive_after_days) * 86_400_000)
                .map(|_| AgentArchiveReason::Expired),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRosterPolicy {
    archive_after_days: u16,
}
impl AgentRosterPolicy {
    pub const fn new(days: u16) -> Option<Self> {
        if matches!(days, 1 | 7 | 30) {
            Some(Self {
                archive_after_days: days,
            })
        } else {
            None
        }
    }
    pub const fn archive_after_days(self) -> u16 {
        self.archive_after_days
    }
}
impl Default for AgentRosterPolicy {
    fn default() -> Self {
        Self {
            archive_after_days: DEFAULT_ARCHIVE_AFTER_DAYS,
        }
    }
}
