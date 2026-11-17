//! 分享页放实况（`specs/redesign/project-rooms.md`“分享页放实况”，3P-6a-2）：`GET /r/<码>/live`。
//!
//! 和围观公开大厅一样：有人来看、快照超过 3 秒才以建房间的应用服务账号重读最近的消息，经本机 Bridge
//! 的解析和验签，去掉撤回、隐藏的，人和 Agent 换成快照内部的编号；有人正在重读时别的请求先拿上一份，
//! 读不到时旧快照最多再给 1 分钟，一次最多等 8 秒。不同的几样：
//!
//! - 按分享码找房间，每次请求都看它此刻还放不放：库里查到的（放或者不放）在内存里最多留 5 秒。不放了，
//!   这间房的快照扔掉。
//! - 房间状态 30 秒读一次（大房间的状态大）：隐藏了哪些、在线数、说话的 Agent 是谁的。状态里有
//!   `m.room.encryption` 就当不放，这一间以后都回 404。
//! - 只给开实况那一刻以后说的、最近 20 句，每句开头 280 个字（[`live_view`](super::live_view)）。
//! - 最多同时留 200 间房的快照，久没人看的先扔；同时重读的不超过 4 间。没人看的房间不读。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use agent_room_application::{
    persistence::RepositoryError,
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock, LiveRoomLookup,
        LiveRoomRecord, MatrixFailure, MatrixTimelineEvent, NetworkAgentLookup, PortFuture,
        PublicLobbyMatrixReader, RoomAgentKind, RoomPeopleDirectory, SecretFactory,
        SecretGenerationFailure,
    },
    public_room_directory::{
        PublicDirectoryFailureKind, PublicRoomDirectoryUseCases, PublicRoomOnline,
    },
    room_people::shown_display_name,
};
use agent_room_bridge_core::agent_verification::{
    AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
};
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use axum::body::Bytes;

use super::{
    READ_EVENTS, REFRESH_DEADLINE, REFRESH_INTERVAL_MS, STALE_LIMIT_MS, agent_kinds::AgentKinds,
    live_view, reading, view,
};

/// 按分享码查到的“放不放”在内存里留这么久。
const ELIGIBILITY_TTL_MS: i64 = 5_000;
/// 记着的分享码最多这么多个，满了先清掉过期的，还满就全清。
const MAX_ELIGIBILITY_ENTRIES: usize = 4_096;
/// 房间状态隔这么久才重读一次。
const STATE_INTERVAL_MS: i64 = 30_000;
/// 最多同时留这么多间房的快照。
const MAX_ROOMS: usize = 200;
/// 最多同时重读这么多间房。
const MAX_CONCURRENT_REFRESHES: usize = 4;
const ENCRYPTION_EVENT_TYPE: &str = "m.room.encryption";

pub(crate) struct RoomLiveWatchDependencies {
    /// 按分享码找放着实况的房间（库里）。
    pub(crate) rooms: Arc<dyn LiveRoomLookup>,
    /// 在线数：和分享页卡片上的是同一个。
    pub(crate) directory: Arc<dyn PublicRoomDirectoryUseCases>,
    /// 说话的本机 Agent 是谁的。
    pub(crate) people: Arc<dyn RoomPeopleDirectory>,
    pub(crate) reader: Arc<dyn PublicLobbyMatrixReader>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) network_agents: Arc<dyn NetworkAgentLookup>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
    pub(crate) clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomLiveFailure {
    /// 这间房此刻不放实况，或者没有这个码：不分是哪一种。
    NotLive,
    /// 读不到，也没有不太旧的快照可给。
    Unavailable,
}

/// 分享页用到的几样：页面上放不放那一块，和 `/live` 的回答。
pub(crate) trait RoomLiveSource: Send + Sync {
    /// 5 秒内查过这个码（放或者不放）：这次不用去库里查。
    fn knows(&self, share_code: &RoomShareCode) -> bool;

    /// 此刻放不放这间房的实况。查不了当成不放。
    fn is_live<'a>(&'a self, share_code: &'a RoomShareCode) -> PortFuture<'a, bool>;

    /// 实况，已经是回给网页的 JSON。
    fn snapshot<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, Result<Bytes, RoomLiveFailure>>;
}

pub(crate) struct RoomLiveWatch {
    rooms: Arc<dyn LiveRoomLookup>,
    directory: Arc<dyn PublicRoomDirectoryUseCases>,
    people: Arc<dyn RoomPeopleDirectory>,
    reader: Arc<dyn PublicLobbyMatrixReader>,
    clock: Arc<dyn Clock>,
    authenticator: Arc<AgentInstanceMessageAuthenticator>,
    keys: view::Keys,
    network_agents: AgentKinds,
    eligibility: Mutex<HashMap<RoomShareCode, Eligibility>>,
    slots: Mutex<Slots>,
    refreshes: tokio::sync::Semaphore,
}

impl RoomLiveWatch {
    /// # Errors
    ///
    /// 生成快照编号用的随机盐失败时返回错误。
    pub(crate) fn new(
        dependencies: RoomLiveWatchDependencies,
    ) -> Result<Self, SecretGenerationFailure> {
        let keys = view::Keys::new(dependencies.secrets)?;
        let authenticator = Arc::new(AgentInstanceMessageAuthenticator::new(
            AgentInstanceMessageAuthenticatorDependencies {
                verification: Arc::new(reading::CachedVerification::new(
                    dependencies.verification,
                    dependencies.clock.clone(),
                )),
                signatures: dependencies.signatures,
            },
        ));
        Ok(Self {
            rooms: dependencies.rooms,
            directory: dependencies.directory,
            people: dependencies.people,
            reader: dependencies.reader,
            clock: dependencies.clock,
            authenticator,
            keys,
            network_agents: AgentKinds::new(dependencies.network_agents),
            eligibility: Mutex::new(HashMap::new()),
            slots: Mutex::new(Slots::default()),
            refreshes: tokio::sync::Semaphore::new(MAX_CONCURRENT_REFRESHES),
        })
    }

    async fn snapshot_of(&self, share_code: &RoomShareCode) -> Result<Bytes, RoomLiveFailure> {
        let room = self
            .live_room(share_code)
            .await?
            .ok_or(RoomLiveFailure::NotLive)?;
        let slot = self.slot(&room);
        if slot.state().sealed {
            return Err(RoomLiveFailure::NotLive);
        }
        let now = self.clock.now();
        if !slot.due(now) {
            return slot.usable(now).ok_or(RoomLiveFailure::Unavailable);
        }
        let Ok(_refreshing) = slot.refresh.try_lock() else {
            // 别的请求正在重读：有不太旧的就先给它，没有就等它读完。
            if let Some(body) = slot.usable(now) {
                return Ok(body);
            }
            if let Ok(finished) = tokio::time::timeout(REFRESH_DEADLINE, slot.refresh.lock()).await
            {
                drop(finished);
            }
            return slot.answer(self.clock.now());
        };
        let now = self.clock.now();
        if !slot.due(now) {
            return slot.answer(now);
        }
        // 同时重读的房间不超过 4 间；等不到的先给旧的。
        let Ok(Ok(_permit)) =
            tokio::time::timeout(REFRESH_DEADLINE, self.refreshes.acquire()).await
        else {
            return slot.usable(now).ok_or(RoomLiveFailure::Unavailable);
        };
        let now = self.clock.now();
        slot.attempted(now);
        match tokio::time::timeout(
            REFRESH_DEADLINE,
            self.refresh(share_code, &room, &slot, now),
        )
        .await
        {
            Ok(Ok(Refreshed::Body(body))) => {
                slot.store(now, body.clone());
                Ok(body)
            }
            Ok(Ok(Refreshed::Gone)) => {
                self.forget(share_code, &room);
                Err(RoomLiveFailure::NotLive)
            }
            Ok(Ok(Refreshed::Sealed)) => {
                tracing::warn!("分享页放实况：目录记着不加密，聊天服务器上却加了密，不放");
                slot.seal();
                Err(RoomLiveFailure::NotLive)
            }
            Ok(Err(failure)) => {
                failure.log();
                slot.usable(now).ok_or(RoomLiveFailure::Unavailable)
            }
            Err(_) => {
                tracing::warn!("分享页放实况：读得太久，这次放弃");
                slot.usable(now).ok_or(RoomLiveFailure::Unavailable)
            }
        }
    }

    /// 这个码此刻放着实况的房间。5 秒内查过就用上次的；放的房间变了（不放了、换了一刻），上一份快照扔掉。
    async fn live_room(
        &self,
        share_code: &RoomShareCode,
    ) -> Result<Option<LiveRoomRecord>, RoomLiveFailure> {
        let now = self.clock.now();
        if let Some(entry) = self.eligibility().get(share_code)
            && fresh(entry.checked_at, now)
        {
            return Ok(entry.room.clone());
        }
        let found = self.rooms.live_room(share_code).await.map_err(|error| {
            RefreshFailure::Lookup(error).log();
            RoomLiveFailure::Unavailable
        })?;
        let previous = {
            let mut eligibility = self.eligibility();
            if eligibility.len() >= MAX_ELIGIBILITY_ENTRIES {
                eligibility.retain(|_, entry| fresh(entry.checked_at, now));
                if eligibility.len() >= MAX_ELIGIBILITY_ENTRIES {
                    eligibility.clear();
                }
            }
            eligibility
                .insert(
                    share_code.clone(),
                    Eligibility {
                        checked_at: now,
                        room: found.clone(),
                    },
                )
                .and_then(|entry| entry.room)
        };
        if let Some(previous) = previous
            && found.as_ref() != Some(&previous)
        {
            self.slots().remove(&SlotKey::of(&previous));
        }
        Ok(found)
    }

    /// 读的时候发现房间已经不公开了：记成不放，快照扔掉。
    fn forget(&self, share_code: &RoomShareCode, room: &LiveRoomRecord) {
        self.eligibility().insert(
            share_code.clone(),
            Eligibility {
                checked_at: self.clock.now(),
                room: None,
            },
        );
        self.slots().remove(&SlotKey::of(room));
    }

    fn slot(&self, room: &LiveRoomRecord) -> Arc<RoomSlot> {
        self.slots().get_or_insert(SlotKey::of(room))
    }

    async fn refresh(
        &self,
        share_code: &RoomShareCode,
        room: &LiveRoomRecord,
        slot: &RoomSlot,
        now: UtcMillis,
    ) -> Result<Refreshed, RefreshFailure> {
        let room_id = &room.matrix_room_id;
        let events = if slot.state_due(now) {
            let (events, state) = tokio::try_join!(
                self.reader.recent_messages(room_id, READ_EVENTS),
                self.reader.current_state(room_id),
            )
            .map_err(RefreshFailure::Matrix)?;
            if encrypted(&state) {
                return Ok(Refreshed::Sealed);
            }
            let Some(online) = self.online(share_code).await? else {
                return Ok(Refreshed::Gone);
            };
            slot.store_state(now, reading::hidden_events(&state), online);
            events
        } else {
            self.reader
                .recent_messages(room_id, READ_EVENTS)
                .await
                .map_err(RefreshFailure::Matrix)?
        };
        let mutations = reading::verified_messages(self.authenticator.clone(), room_id, events)
            .await
            .map_err(RefreshFailure::Reading)?;
        let authors = live_view::agent_authors(&mutations);
        let network_agents = self.network_agents.network_agent_ids(&authors).await;
        self.learn_owners(room.catalog_id, slot, &authors, &network_agents)
            .await;
        let (hidden, online, owners) = slot.room_facts();
        let reading = live_view::LiveReading {
            mutations: &mutations,
            hidden: &hidden,
            network_agents: &network_agents,
            owners: &owners,
            online,
            enabled_at: room.enabled_at,
        };
        let view = live_view::assemble(&reading, &self.keys, now);
        serde_json::to_vec(&view)
            .map(|body| Refreshed::Body(Bytes::from(body)))
            .map_err(|_| RefreshFailure::Encode)
    }

    /// 在线数，和分享页卡片上的同一个。外层是“房间还公开着”：目录说没有这间房时是 `None`。
    async fn online(
        &self,
        share_code: &RoomShareCode,
    ) -> Result<Option<Option<PublicRoomOnline>>, RefreshFailure> {
        match self
            .directory
            .detail(None, share_code.as_str().to_owned())
            .await
        {
            Ok(detail) => Ok(Some(detail.online)),
            Err(failure) => match failure.kind() {
                PublicDirectoryFailureKind::NotFound
                | PublicDirectoryFailureKind::InvalidRequest => Ok(None),
                PublicDirectoryFailureKind::DependencyUnavailable
                | PublicDirectoryFailureKind::Internal => Err(RefreshFailure::Directory),
            },
        }
    }

    /// 说话的本机 Agent 里还不知道是谁的，问一次库；记到下次读房间状态。问不了的这次不写主人。
    async fn learn_owners(
        &self,
        catalog_id: RoomCatalogId,
        slot: &RoomSlot,
        authors: &[AgentId],
        network_agents: &HashSet<AgentId>,
    ) {
        let unknown: Vec<AgentId> = {
            let state = slot.state();
            authors
                .iter()
                .filter(|id| !network_agents.contains(id) && !state.owners.contains_key(id))
                .copied()
                .collect()
        };
        if unknown.is_empty() {
            return;
        }
        match self.people.agent_owners(catalog_id, &unknown).await {
            Ok(records) => {
                let mut found: HashMap<AgentId, Option<String>> = records
                    .into_iter()
                    .map(|record| {
                        let owner = match record.kind {
                            RoomAgentKind::Local => record
                                .owner
                                .map(|owner| shown_display_name(&owner.display_name)),
                            RoomAgentKind::Network => None,
                        };
                        (record.agent_id, owner)
                    })
                    .collect();
                let mut state = slot.state();
                for id in unknown {
                    // 不在房间里的不回答：记成没有主人，到下次读房间状态再问。
                    state.owners.insert(id, found.remove(&id).flatten());
                }
            }
            Err(error) => tracing::warn!(
                operation = error.operation(),
                failure = ?error.kind(),
                "分享页放实况：查不了 Agent 是谁的"
            ),
        }
    }

    fn eligibility(&self) -> MutexGuard<'_, HashMap<RoomShareCode, Eligibility>> {
        self.eligibility
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn slots(&self) -> MutexGuard<'_, Slots> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RoomLiveSource for RoomLiveWatch {
    fn knows(&self, share_code: &RoomShareCode) -> bool {
        let now = self.clock.now();
        self.eligibility()
            .get(share_code)
            .is_some_and(|entry| fresh(entry.checked_at, now))
    }

    fn is_live<'a>(&'a self, share_code: &'a RoomShareCode) -> PortFuture<'a, bool> {
        Box::pin(async move {
            match self.live_room(share_code).await {
                Ok(Some(room)) => !self.slots().is_sealed(&SlotKey::of(&room)),
                Ok(None) | Err(_) => false,
            }
        })
    }

    fn snapshot<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, Result<Bytes, RoomLiveFailure>> {
        Box::pin(self.snapshot_of(share_code))
    }
}

fn fresh(checked_at: UtcMillis, now: UtcMillis) -> bool {
    now.value().saturating_sub(checked_at.value()) < ELIGIBILITY_TTL_MS
}

/// 房间状态里有 `m.room.encryption`：不管里面写的什么算法，都算加密。分享一段对话（`excerpts.rs`）也用它。
pub(super) fn encrypted(state: &[MatrixTimelineEvent]) -> bool {
    state.iter().any(|event| {
        event.event_type().as_str() == ENCRYPTION_EVENT_TYPE && event.state_key() == Some("")
    })
}

struct Eligibility {
    checked_at: UtcMillis,
    /// 放着实况的房间；不放是 `None`。
    room: Option<LiveRoomRecord>,
}

/// 哪间房的哪一次打开：再开一次是新的一刻，快照另起。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SlotKey(RoomCatalogId, i64);

impl SlotKey {
    fn of(room: &LiveRoomRecord) -> Self {
        Self(room.catalog_id, room.enabled_at.value())
    }
}

/// 各间房的快照，按最近一次有人看的先后扔。
#[derive(Default)]
struct Slots {
    rooms: HashMap<SlotKey, (u64, Arc<RoomSlot>)>,
    /// 每有人看一次加一，记在那间房上：最小的就是最久没人看的。
    uses: u64,
}

impl Slots {
    fn get_or_insert(&mut self, key: SlotKey) -> Arc<RoomSlot> {
        self.uses += 1;
        let uses = self.uses;
        if let Some((used, slot)) = self.rooms.get_mut(&key) {
            *used = uses;
            return slot.clone();
        }
        if self.rooms.len() >= MAX_ROOMS
            && let Some(oldest) = self
                .rooms
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(key, _)| *key)
        {
            self.rooms.remove(&oldest);
        }
        let slot = Arc::new(RoomSlot::default());
        self.rooms.insert(key, (uses, slot.clone()));
        slot
    }

    fn remove(&mut self, key: &SlotKey) {
        self.rooms.remove(key);
    }

    fn is_sealed(&self, key: &SlotKey) -> bool {
        self.rooms
            .get(key)
            .is_some_and(|(_, slot)| slot.state().sealed)
    }
}

/// 一间房的快照、“正在重读”的锁，和 30 秒读一次的房间状态。
#[derive(Default)]
struct RoomSlot {
    refresh: tokio::sync::Mutex<()>,
    state: Mutex<RoomSlotState>,
}

#[derive(Default)]
struct RoomSlotState {
    /// 快照读的时刻，和回给网页的 JSON。
    snapshot: Option<(UtcMillis, Bytes)>,
    /// 上次开始重读的时刻，读失败也算。
    attempted_at: Option<UtcMillis>,
    /// 上次读房间状态的时刻；读出来的隐藏消息和在线数。
    state_read_at: Option<UtcMillis>,
    hidden: HashSet<String>,
    online: Option<PublicRoomOnline>,
    /// 说过话的本机 Agent 是谁的，读房间状态时清掉重问。
    owners: HashMap<AgentId, Option<String>>,
    /// 聊天服务器上加了密：这一间以后都不放。
    sealed: bool,
}

impl RoomSlot {
    fn due(&self, now: UtcMillis) -> bool {
        self.state().attempted_at.is_none_or(|attempted_at| {
            now.value().saturating_sub(attempted_at.value()) >= REFRESH_INTERVAL_MS
        })
    }

    fn state_due(&self, now: UtcMillis) -> bool {
        self.state()
            .state_read_at
            .is_none_or(|read_at| now.value().saturating_sub(read_at.value()) >= STATE_INTERVAL_MS)
    }

    fn usable(&self, now: UtcMillis) -> Option<Bytes> {
        self.state()
            .snapshot
            .as_ref()
            .filter(|(taken_at, _)| now.value().saturating_sub(taken_at.value()) <= STALE_LIMIT_MS)
            .map(|(_, body)| body.clone())
    }

    /// 等别人读完以后的回答：加了密的说不放，有不太旧的给它。
    fn answer(&self, now: UtcMillis) -> Result<Bytes, RoomLiveFailure> {
        if self.state().sealed {
            return Err(RoomLiveFailure::NotLive);
        }
        self.usable(now).ok_or(RoomLiveFailure::Unavailable)
    }

    fn attempted(&self, now: UtcMillis) {
        self.state().attempted_at = Some(now);
    }

    fn store(&self, taken_at: UtcMillis, body: Bytes) {
        self.state().snapshot = Some((taken_at, body));
    }

    fn store_state(
        &self,
        read_at: UtcMillis,
        hidden: HashSet<String>,
        online: Option<PublicRoomOnline>,
    ) {
        let mut state = self.state();
        state.state_read_at = Some(read_at);
        state.hidden = hidden;
        state.online = online;
        state.owners.clear();
    }

    fn room_facts(
        &self,
    ) -> (
        HashSet<String>,
        Option<PublicRoomOnline>,
        HashMap<AgentId, Option<String>>,
    ) {
        let state = self.state();
        (state.hidden.clone(), state.online, state.owners.clone())
    }

    fn seal(&self) {
        let mut state = self.state();
        state.sealed = true;
        state.snapshot = None;
    }

    fn state(&self) -> MutexGuard<'_, RoomSlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

enum Refreshed {
    Body(Bytes),
    /// 目录说房间已经不公开了。
    Gone,
    /// 聊天服务器上加了密。
    Sealed,
}

/// 一次重读没成的原因，只记日志。日志里不写分享码和房间名。
enum RefreshFailure {
    Lookup(RepositoryError),
    Directory,
    Matrix(MatrixFailure),
    Reading(reading::ReadingFailure),
    Encode,
}

impl RefreshFailure {
    fn log(&self) {
        match self {
            Self::Lookup(error) => tracing::warn!(
                operation = error.operation(),
                failure = ?error.kind(),
                "分享页放实况：查不了放不放"
            ),
            Self::Directory => tracing::warn!("分享页放实况：目录暂时读不到在线数"),
            Self::Matrix(failure) => tracing::warn!(
                operation = ?failure.operation(),
                failure = ?failure.kind(),
                "分享页放实况：Synapse 没读出来"
            ),
            Self::Reading(failure) => {
                tracing::warn!(failure = ?failure, "分享页放实况：这次读到的验不了");
            }
            Self::Encode => tracing::warn!("分享页放实况：快照写不成 JSON"),
        }
    }
}
