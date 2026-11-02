//! 详情里的“现在几个在线”：以应用服务问房间里每个人和每个 Agent 的在线状态，离开的也算在线（连着、
//! 没在等消息）。一次只算一间，结果留 60 秒；人很多的房间最多问 200 个，写“至少 N 个”。
//!
//! 只是给人看的一个数，问不到不挡详情：库读不了、一个也没问到时回 `None`，问到一部分时按问到的算、
//! 记成“至少”。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState, ids::RoomCatalogId, time::UtcMillis,
};
use futures_util::{StreamExt, stream};

use crate::ports::{PublicRoomDirectoryStore, PublicRoomPresenceMatrix};

/// 一间房最多问几个人。
pub const MAX_PRESENCE_CHECKS: u32 = 200;

/// 一间房算出来的数留多久。
pub const ONLINE_COUNT_TTL_MILLIS: i64 = 60_000;

/// 同时问几个。
const PRESENCE_CONCURRENCY: usize = 8;

/// 最多记着几间房的数；超过时先丢过期的，还多就全丢。
const MAX_CACHED_ROOMS: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicRoomOnline {
    pub count: u32,
    /// 没问全（人太多，或者有几个没问到），真实的数可能更多。
    pub at_least: bool,
}

pub(super) struct OnlineCounter {
    directory: Arc<dyn PublicRoomDirectoryStore>,
    presence: Arc<dyn PublicRoomPresenceMatrix>,
    cache: Mutex<HashMap<RoomCatalogId, Counted>>,
}

/// 一间房上一次算的结果，问不到的也记着，免得一直去问。
#[derive(Clone, Copy)]
struct Counted {
    at: UtcMillis,
    online: Option<PublicRoomOnline>,
}

impl OnlineCounter {
    pub(super) fn new(
        directory: Arc<dyn PublicRoomDirectoryStore>,
        presence: Arc<dyn PublicRoomPresenceMatrix>,
    ) -> Self {
        Self {
            directory,
            presence,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub(super) async fn count(
        &self,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> Option<PublicRoomOnline> {
        if let Some(cached) = self.cached(catalog_id, now) {
            return cached.online;
        }
        let online = self.ask(catalog_id).await;
        self.remember(catalog_id, Counted { at: now, online });
        online
    }

    fn cached(&self, catalog_id: RoomCatalogId, now: UtcMillis) -> Option<Counted> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .get(&catalog_id)
            .copied()
            .filter(|counted| fresh(counted.at, now))
    }

    fn remember(&self, catalog_id: RoomCatalogId, counted: Counted) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= MAX_CACHED_ROOMS {
            cache.retain(|_, kept| fresh(kept.at, counted.at));
            if cache.len() >= MAX_CACHED_ROOMS {
                cache.clear();
            }
        }
        cache.insert(catalog_id, counted);
    }

    async fn ask(&self, catalog_id: RoomCatalogId) -> Option<PublicRoomOnline> {
        // 多拿一个，看是不是没问全。
        let mut users = self
            .directory
            .public_room_presence_candidates(catalog_id, MAX_PRESENCE_CHECKS + 1)
            .await
            .ok()?;
        let more = users.len() > MAX_PRESENCE_CHECKS as usize;
        users.truncate(MAX_PRESENCE_CHECKS as usize);
        if users.is_empty() {
            return Some(PublicRoomOnline {
                count: 0,
                at_least: false,
            });
        }
        // 按值交给每个请求：借着 `users` 的闭包放进 `buffer_unordered`，整个 future 就证明不了对
        // 任意生命周期都 `Send`。
        let answers: Vec<_> = stream::iter(users)
            .map(|user| async move { self.presence.presence(&user).await })
            .buffer_unordered(PRESENCE_CONCURRENCY)
            .collect()
            .await;
        let answered = answers.iter().filter(|answer| answer.is_ok()).count();
        if answered == 0 {
            return None;
        }
        let online = answers
            .iter()
            .filter(|answer| matches!(answer, Ok(state) if *state != MatrixPresenceState::Offline))
            .count();
        Some(PublicRoomOnline {
            count: u32::try_from(online).unwrap_or(u32::MAX),
            at_least: more || answered < answers.len(),
        })
    }
}

fn fresh(at: UtcMillis, now: UtcMillis) -> bool {
    now.value().saturating_sub(at.value()) < ONLINE_COUNT_TTL_MILLIS
}
