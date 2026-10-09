//! Connection evidence and recoverable roster archival are independent of task completion.
use crate::agent_status::AgentWorkStatus;

pub const RECONNECT_GRACE_MS: i64 = 30_000;
pub const RECEPTION_FRESHNESS_MS: i64 = 15_000;
/// `waitingUntil` 最多比事件发布时间晚多少。比连接租约的续租间隔（约 2 分钟）长，
/// 一直等着的 Agent 跟着续租就能保持“持续等待消息”，不必为它另发状态。
pub const WAITING_LEASE_MS: i64 = 180_000;
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
/// Matrix 自带的在线状态（presence），按用户算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixPresenceState {
    Online,
    Unavailable,
    Offline,
}
impl MatrixPresenceState {
    /// Matrix 的 `presence` 字段。只认这三种，别的（比如没开的 `busy`）不认。
    pub fn from_matrix(value: &str) -> Option<Self> {
        match value {
            "online" => Some(Self::Online),
            "unavailable" => Some(Self::Unavailable),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }
}
/// 读的一边对一个 Agent 的 Matrix 在线状态知道多少。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixPresenceObservation {
    pub state: MatrixPresenceState,
    /// 亲眼看到它从在线或离开变成离线的时刻。
    pub offline_seen_at: Option<i64>,
    /// 在线状态里的“上次活动”：拿到时的时刻减去 `last_active_ago`。
    pub last_active_at: Option<i64>,
}
/// 在不在线从哪里看。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLiveness {
    /// 旧写法：状态事件自带租约，按租约判断。
    Lease,
    /// 名片（`liveness: "presence"`）：状态事件只说是谁，在不在线、在不在等消息都看
    /// Matrix 的在线状态；还没拿到就是 `None`。
    Presence(Option<MatrixPresenceObservation>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentPresenceEvidence {
    pub reported_status: AgentWorkStatus,
    pub lease_expires_at: i64,
    /// 状态事件的时间；名片就是写名片的时间。
    pub last_active_at: i64,
    pub last_polled_at: Option<i64>,
    pub listening_until: Option<i64>,
    pub reception_known: bool,
    pub liveness: AgentLiveness,
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
        let (connection, reception, offline_since) = match self.liveness {
            AgentLiveness::Lease => self.lease_state(now),
            AgentLiveness::Presence(observation) => self.presence_state(observation, now),
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

    fn lease_state(self, now: i64) -> (AgentConnection, AgentReception, Option<i64>) {
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
        (connection, reception, offline_since)
    }

    /// 名片按在线状态判断：在线就是在等消息，离开是连着没在等，离线或还没拿到都算离线。
    /// 不看名片里的工作状态和租约，也没有“重连中”：Synapse 自己已经等了约 30 秒才说离线。
    fn presence_state(
        self,
        observation: Option<MatrixPresenceObservation>,
        now: i64,
    ) -> (AgentConnection, AgentReception, Option<i64>) {
        match observation.map(|observed| observed.state) {
            Some(MatrixPresenceState::Online) => {
                (AgentConnection::Online, AgentReception::Waiting, None)
            }
            Some(MatrixPresenceState::Unavailable) => {
                (AgentConnection::Online, AgentReception::OnResume, None)
            }
            Some(MatrixPresenceState::Offline) | None => {
                // 看到它变离线的，从那一刻算；没看到就用在线状态里的上次活动，但不早于
                // 名片本身（写名片时它一定在）。都不晚于现在。
                let since = observation
                    .and_then(|observed| observed.offline_seen_at)
                    .unwrap_or_else(|| {
                        observation
                            .and_then(|observed| observed.last_active_at)
                            .map_or(self.last_active_at, |at| at.max(self.last_active_at))
                    })
                    .min(now);
                (
                    AgentConnection::Offline,
                    AgentReception::Unavailable,
                    Some(since),
                )
            }
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
