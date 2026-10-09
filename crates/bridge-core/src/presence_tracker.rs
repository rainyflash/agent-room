//! 写名片的 Agent 的 Matrix 在线状态（`specs/agent-liveness/design.md`）。
//!
//! 只认这次连上以后从服务器拿到的：同步里的 `m.presence`，和按需问到的。全量同步只带
//! 不离线的人，接着同步只带变了的人；写名片、却没拿到的 Agent 另外各问一次。
use std::collections::BTreeMap;

use agent_room_application::ports::{MatrixUserId, MatrixUserPresence};
use agent_room_domain::{
    agent_lifecycle::{MatrixPresenceObservation, MatrixPresenceState},
    time::UtcMillis,
};

/// 上次确认它不离线以后隔了这么久（电脑睡着、断网），就不算一直看着：这段时间里变成离线的，
/// 不知道是哪一刻变的，交给在线状态里的“上次活动”。同步每 30 秒左右回来一次。
pub const WATCHING_GAP_MS: i64 = 120_000;
/// 问失败了，隔多久才再问同一个人。
pub const PRESENCE_FETCH_RETRY_MS: i64 = 60_000;

/// 一次同步带回的在线状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixPresenceUpdate {
    observed_at: UtcMillis,
    full_state: bool,
    users: Vec<MatrixUserPresence>,
}

impl MatrixPresenceUpdate {
    pub const fn new(
        observed_at: UtcMillis,
        full_state: bool,
        users: Vec<MatrixUserPresence>,
    ) -> Self {
        Self {
            observed_at,
            full_state,
            users,
        }
    }

    pub const fn observed_at(&self) -> UtcMillis {
        self.observed_at
    }

    /// 全量同步：只带不离线的人。
    pub const fn full_state(&self) -> bool {
        self.full_state
    }

    pub fn users(&self) -> &[MatrixUserPresence] {
        &self.users
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackedPresence {
    observation: MatrixPresenceObservation,
    /// 最近一次确认它不离线：收到在线或离开，或者之后每次同步成功。离线时没有。
    confirmed_at: Option<i64>,
}

/// 按用户记的在线状态。一份投影只跟着一个连接的同步走。
#[derive(Debug, Clone, Default)]
pub struct MatrixPresenceTracker {
    entries: BTreeMap<MatrixUserId, TrackedPresence>,
    failed_at: BTreeMap<MatrixUserId, i64>,
}

impl MatrixPresenceTracker {
    /// 记下一次同步带回的在线状态。
    ///
    /// 全量同步只带不离线的人：之前记下的都作废（离开的这段时间里谁变了不知道），没带的
    /// 另外去问。接着同步时，先按上一次确认的时刻记下这次变了的人，再确认没变的人还是原来
    /// 的样子。
    pub fn apply_sync(&mut self, update: &MatrixPresenceUpdate) {
        let at = update.observed_at().value();
        if update.full_state() {
            self.entries.clear();
        }
        for presence in update.users() {
            self.record(presence, at);
        }
        for entry in self.entries.values_mut() {
            if entry.observation.state != MatrixPresenceState::Offline {
                entry.confirmed_at = Some(at);
            }
        }
    }

    /// 记下问到的在线状态，`None` 是没问到。问的时候同步里来了新的，就以同步的为准。
    pub fn record_fetched(
        &mut self,
        user_id: &MatrixUserId,
        presence: Option<&MatrixUserPresence>,
        at: UtcMillis,
    ) {
        let Some(presence) = presence else {
            self.failed_at.insert(user_id.clone(), at.value());
            return;
        };
        self.failed_at.remove(user_id);
        if !self.entries.contains_key(user_id) {
            self.record(presence, at.value());
        }
    }

    pub fn observation(&self, user_id: &MatrixUserId) -> Option<MatrixPresenceObservation> {
        self.entries.get(user_id).map(|entry| entry.observation)
    }

    /// 还没拿到、也不在问失败后的冷却里：该去问了。
    pub fn wants(&self, user_id: &MatrixUserId, now: UtcMillis) -> bool {
        !self.entries.contains_key(user_id)
            && self
                .failed_at
                .get(user_id)
                .is_none_or(|failed| now.value() - failed >= PRESENCE_FETCH_RETRY_MS)
    }

    fn record(&mut self, presence: &MatrixUserPresence, at: i64) {
        let last_active_at = presence
            .last_active_ago_ms()
            .and_then(|ago| i64::try_from(ago).ok())
            .map(|ago| at.saturating_sub(ago));
        let state = presence.state();
        let previous = self.entries.get(presence.user_id());
        let tracked = if state == MatrixPresenceState::Offline {
            // 一直看着它不离线、这次说它离线了，离线就从这一刻算；没一直看着，不知道是哪一刻变的。
            let offline_seen_at = match previous {
                Some(previous) if previous.observation.state == MatrixPresenceState::Offline => {
                    previous.observation.offline_seen_at
                }
                Some(previous) => previous
                    .confirmed_at
                    .filter(|confirmed| at - confirmed <= WATCHING_GAP_MS)
                    .map(|_| at),
                None => None,
            };
            TrackedPresence {
                observation: MatrixPresenceObservation {
                    state,
                    offline_seen_at,
                    last_active_at,
                },
                confirmed_at: None,
            }
        } else {
            TrackedPresence {
                observation: MatrixPresenceObservation {
                    state,
                    offline_seen_at: None,
                    last_active_at,
                },
                confirmed_at: Some(at),
            }
        };
        self.entries.insert(presence.user_id().clone(), tracked);
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{MatrixUserId, MatrixUserPresence};
    use agent_room_domain::{
        agent_lifecycle::{MatrixPresenceObservation, MatrixPresenceState},
        time::UtcMillis,
    };

    use super::{MatrixPresenceTracker, MatrixPresenceUpdate};

    fn user(name: &str) -> MatrixUserId {
        MatrixUserId::new(format!("@{name}:example.org")).unwrap()
    }

    fn at(value: i64) -> UtcMillis {
        UtcMillis::new(value).unwrap()
    }

    fn reported(name: &str, state: MatrixPresenceState, ago: Option<u64>) -> MatrixUserPresence {
        MatrixUserPresence::new(user(name), state, ago)
    }

    fn sync(
        observed_at: i64,
        full_state: bool,
        users: Vec<MatrixUserPresence>,
    ) -> MatrixPresenceUpdate {
        MatrixPresenceUpdate::new(at(observed_at), full_state, users)
    }

    #[test]
    fn 一直看着它在线_同步说离线就从那一刻算() {
        let mut tracker = MatrixPresenceTracker::default();
        tracker.apply_sync(&sync(
            1_000_000,
            true,
            vec![reported("ada", MatrixPresenceState::Online, Some(0))],
        ));
        tracker.apply_sync(&sync(1_030_000, false, Vec::new()));
        tracker.apply_sync(&sync(
            1_060_000,
            false,
            vec![reported("ada", MatrixPresenceState::Offline, Some(40_000))],
        ));
        assert_eq!(
            tracker.observation(&user("ada")),
            Some(MatrixPresenceObservation {
                state: MatrixPresenceState::Offline,
                offline_seen_at: Some(1_060_000),
                last_active_at: Some(1_020_000),
            })
        );

        // 之后再说一次离线，还是看到它变离线的那一刻。
        tracker.apply_sync(&sync(
            1_090_000,
            false,
            vec![reported("ada", MatrixPresenceState::Offline, Some(70_000))],
        ));
        assert_eq!(
            tracker
                .observation(&user("ada"))
                .and_then(|observed| observed.offline_seen_at),
            Some(1_060_000)
        );
    }

    #[test]
    fn 中间断了两分钟以上_离线的那一刻不算看到() {
        let mut tracker = MatrixPresenceTracker::default();
        tracker.apply_sync(&sync(
            1_000_000,
            true,
            vec![reported("ada", MatrixPresenceState::Unavailable, None)],
        ));
        // 电脑睡了一个小时，醒来以后的同步才说它离线。
        tracker.apply_sync(&sync(
            4_600_000,
            false,
            vec![reported(
                "ada",
                MatrixPresenceState::Offline,
                Some(3_000_000),
            )],
        ));
        assert_eq!(
            tracker.observation(&user("ada")),
            Some(MatrixPresenceObservation {
                state: MatrixPresenceState::Offline,
                offline_seen_at: None,
                last_active_at: Some(1_600_000),
            })
        );
    }

    #[test]
    fn 全量同步作废之前记下的_没带的要去问() {
        let mut tracker = MatrixPresenceTracker::default();
        tracker.apply_sync(&sync(
            1_000_000,
            true,
            vec![
                reported("ada", MatrixPresenceState::Online, None),
                reported("bob", MatrixPresenceState::Online, None),
            ],
        ));
        assert!(!tracker.wants(&user("ada"), at(1_000_000)));
        // 重新连上：bob 在这期间离线了，全量同步里没有他。
        tracker.apply_sync(&sync(
            2_000_000,
            true,
            vec![reported("ada", MatrixPresenceState::Online, None)],
        ));
        assert_eq!(tracker.observation(&user("bob")), None);
        assert!(tracker.wants(&user("bob"), at(2_000_000)));
        assert!(!tracker.wants(&user("ada"), at(2_000_000)));
    }

    #[test]
    fn 问到的离线不算看到_问失败一分钟后再问() {
        let mut tracker = MatrixPresenceTracker::default();
        let bob = user("bob");
        tracker.record_fetched(&bob, None, at(1_000_000));
        assert!(!tracker.wants(&bob, at(1_059_999)));
        assert!(tracker.wants(&bob, at(1_060_000)));

        let offline = reported("bob", MatrixPresenceState::Offline, Some(600_000));
        tracker.record_fetched(&bob, Some(&offline), at(1_060_000));
        assert_eq!(
            tracker.observation(&bob),
            Some(MatrixPresenceObservation {
                state: MatrixPresenceState::Offline,
                offline_seen_at: None,
                last_active_at: Some(460_000),
            })
        );
        assert!(!tracker.wants(&bob, at(1_200_000)));
    }

    #[test]
    fn 问的时候同步里来了新的_以同步的为准() {
        let mut tracker = MatrixPresenceTracker::default();
        let ada = user("ada");
        tracker.apply_sync(&sync(
            1_000_000,
            false,
            vec![reported("ada", MatrixPresenceState::Online, None)],
        ));
        let stale = reported("ada", MatrixPresenceState::Offline, Some(5_000));
        tracker.record_fetched(&ada, Some(&stale), at(1_000_100));
        assert_eq!(
            tracker.observation(&ada).map(|observed| observed.state),
            Some(MatrixPresenceState::Online)
        );
    }

    #[test]
    fn 问到在线以后一直同步_变离线算看到() {
        let mut tracker = MatrixPresenceTracker::default();
        let ada = user("ada");
        let online = reported("ada", MatrixPresenceState::Online, Some(0));
        tracker.record_fetched(&ada, Some(&online), at(1_000_000));
        tracker.apply_sync(&sync(1_030_000, false, Vec::new()));
        tracker.apply_sync(&sync(
            1_050_000,
            false,
            vec![reported("ada", MatrixPresenceState::Offline, Some(30_000))],
        ));
        assert_eq!(
            tracker
                .observation(&ada)
                .and_then(|observed| observed.offline_seen_at),
            Some(1_050_000)
        );
    }
}
