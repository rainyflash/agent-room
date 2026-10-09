//! 不登录也能看公开大厅（specs/public-lobby-watch/design.md 第 1 步）。
//!
//! 有人来看某个大厅、而它的快照已经超过 3 秒时，以建大厅的应用服务账号重读一次：最近的消息和
//! 房间状态，经本机 Bridge 的同一套解析和验签，去掉被隐藏、撤回的，人和 Agent 换成快照内部的
//! 编号。快照放在内存里，请求直接拿；有人正在重读时，别的请求先拿上一份。没人看就不读。

mod reading;
mod view;

#[cfg(test)]
mod real_dependency_tests;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use agent_room_application::{
    network_agents::DEFAULT_LOBBY_SLUG,
    persistence::RepositoryError,
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock, MatrixFailure,
        MatrixRoomId, NetworkAgentLookup, PublicLobbyMatrixReader, RoomDirectory,
        RoomDirectoryQuery, SecretFactory, SecretGenerationFailure,
    },
};
use agent_room_bridge_core::agent_verification::{
    AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
};
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    rooms::{MatrixRoomReference, RoomSlug},
    time::UtcMillis,
};
use axum::body::Bytes;

/// 快照比这旧时，有人来看就重读。
const REFRESH_INTERVAL_MS: i64 = 3_000;
/// 重读一直失败时，旧快照最多再给这么久，之后照实说暂时看不了。
const STALE_LIMIT_MS: i64 = 60_000;
/// 一次重读最多等这么久。还没有快照时，来看的请求要等这次读完。
const REFRESH_DEADLINE: Duration = Duration::from_secs(8);
/// 公开大厅目录在内存里留这么久：认 slug 不用每次查库。
const DIRECTORY_TTL_MS: i64 = 30_000;
/// 每次读最近这么多条消息事件（含修订），从里面留下最近 50 条消息。
const READ_EVENTS: u16 = 60;
/// `/watch/default` 是默认大厅，和网络 Agent 省略房间时进的是同一间。
const DEFAULT_ALIAS: &str = "default";
/// 记着哪些 Agent 是网络 Agent，最多这么多个，满了清空重来。
const MAX_KNOWN_AGENT_KINDS: usize = 10_000;
/// 一次最多问这么多个 Agent 是不是网络 Agent。
const LOOKUP_BATCH: usize = 100;

pub(crate) struct PublicWatchDependencies {
    /// 网络 Agent 的总开关：公开大厅本来就对不要账号的网络 Agent 开放，围观跟着它走。
    pub(crate) enabled: bool,
    pub(crate) directory: Arc<dyn RoomDirectory>,
    pub(crate) reader: Arc<dyn PublicLobbyMatrixReader>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) network_agents: Arc<dyn NetworkAgentLookup>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
    pub(crate) clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicWatchFailure {
    /// 这个部署没开网络 Agent：公开大厅只有登录的人能读，围观也关着。
    Disabled,
    LobbyNotFound,
    /// 读不到，也没有不太旧的快照可给。
    Unavailable,
}

pub(crate) struct PublicWatch {
    enabled: bool,
    directory: Arc<dyn RoomDirectory>,
    reader: Arc<dyn PublicLobbyMatrixReader>,
    network_agents: Arc<dyn NetworkAgentLookup>,
    clock: Arc<dyn Clock>,
    authenticator: Arc<AgentInstanceMessageAuthenticator>,
    keys: view::Keys,
    lobbies: tokio::sync::Mutex<DirectoryCache>,
    slots: Mutex<HashMap<RoomCatalogId, Arc<LobbySlot>>>,
    agent_kinds: Mutex<HashMap<AgentId, bool>>,
}

impl PublicWatch {
    /// # Errors
    ///
    /// 生成快照编号用的随机盐失败时返回错误。
    pub(crate) fn new(
        dependencies: PublicWatchDependencies,
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
            enabled: dependencies.enabled,
            directory: dependencies.directory,
            reader: dependencies.reader,
            network_agents: dependencies.network_agents,
            clock: dependencies.clock,
            authenticator,
            keys,
            lobbies: tokio::sync::Mutex::new(DirectoryCache::default()),
            slots: Mutex::new(HashMap::new()),
            agent_kinds: Mutex::new(HashMap::new()),
        })
    }

    /// 这个公开大厅此刻的样子，已经是要回给网页的 JSON。`slug` 写 `default` 是默认大厅。
    pub(crate) async fn watch(&self, slug: &str) -> Result<Bytes, PublicWatchFailure> {
        if !self.enabled {
            return Err(PublicWatchFailure::Disabled);
        }
        let lobby = self.lobby(slug).await?;
        let slot = self.slot(lobby.catalog_id);
        let now = self.clock.now();
        if !slot.due(now) {
            return slot.usable(now).ok_or(PublicWatchFailure::Unavailable);
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
            return slot
                .usable(self.clock.now())
                .ok_or(PublicWatchFailure::Unavailable);
        };
        // 拿到锁时可能刚有人读完。
        let now = self.clock.now();
        if !slot.due(now) {
            return slot.usable(now).ok_or(PublicWatchFailure::Unavailable);
        }
        slot.attempted(now);
        match tokio::time::timeout(REFRESH_DEADLINE, self.refresh(&lobby, now)).await {
            Ok(Ok(body)) => {
                slot.store(now, body.clone());
                Ok(body)
            }
            Ok(Err(failure)) => {
                failure.log(&lobby.slug);
                slot.usable(now).ok_or(PublicWatchFailure::Unavailable)
            }
            Err(_) => {
                tracing::warn!(lobby = %lobby.slug, "围观公开大厅：读得太久，这次放弃");
                slot.usable(now).ok_or(PublicWatchFailure::Unavailable)
            }
        }
    }

    /// 按 slug 找公开大厅；`default` 在没有叫这个名字的大厅时是默认大厅。
    async fn lobby(&self, slug: &str) -> Result<WatchLobby, PublicWatchFailure> {
        // 不像 slug 的直接说没有，不去碰目录。
        if RoomSlug::new(slug).is_err() {
            return Err(PublicWatchFailure::LobbyNotFound);
        }
        let mut cache = self.lobbies.lock().await;
        let now = self.clock.now();
        if cache.due(now) {
            cache.checked_at = Some(now);
            match self
                .directory
                .list_public(&RoomDirectoryQuery::default())
                .await
            {
                Ok(entries) => {
                    cache.failed = false;
                    cache.lobbies = Some(
                        entries
                            .into_iter()
                            .filter_map(|entry| {
                                Some(WatchLobby {
                                    catalog_id: entry.catalog.id(),
                                    name: entry.catalog.name().to_owned(),
                                    slug: entry.catalog.slug()?.as_str().to_owned(),
                                })
                            })
                            .collect(),
                    );
                }
                Err(error) => {
                    // 有上次查到的就先用着，隔一会儿再查。
                    cache.failed = true;
                    tracing::warn!(
                        operation = error.operation(),
                        failure = ?error.kind(),
                        "围观公开大厅：读不到公开大厅目录"
                    );
                }
            }
        }
        let lobbies = cache
            .lobbies
            .as_deref()
            .ok_or(PublicWatchFailure::Unavailable)?;
        find_lobby(lobbies, slug)
            .cloned()
            .ok_or(PublicWatchFailure::LobbyNotFound)
    }

    fn slot(&self, catalog_id: RoomCatalogId) -> Arc<LobbySlot> {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(catalog_id)
            .or_default()
            .clone()
    }

    async fn refresh(&self, lobby: &WatchLobby, now: UtcMillis) -> Result<Bytes, RefreshFailure> {
        let room = self
            .directory
            .find_public_observation_room(lobby.catalog_id)
            .await
            .map_err(RefreshFailure::Directory)?;
        // 还没人进过的大厅没有分片：人和消息都是空的。
        let reading = match room {
            Some(room) => Some(self.read_room(&room.matrix_room_id, now).await?),
            None => None,
        };
        let view = view::assemble(lobby, reading, &self.keys, now);
        serde_json::to_vec(&view)
            .map(Bytes::from)
            .map_err(|_| RefreshFailure::Encode)
    }

    async fn read_room(
        &self,
        matrix_room_id: &MatrixRoomReference,
        now: UtcMillis,
    ) -> Result<view::RoomReading, RefreshFailure> {
        let room_id =
            MatrixRoomId::new(matrix_room_id.as_str()).map_err(|_| RefreshFailure::InvalidRoom)?;
        let (events, state) = tokio::try_join!(
            self.reader.recent_messages(&room_id, READ_EVENTS),
            self.reader.current_state(&room_id),
        )
        .map_err(RefreshFailure::Matrix)?;
        let hidden = reading::hidden_events(&state);
        let mutations = reading::verified_messages(self.authenticator.clone(), &room_id, events)
            .await
            .map_err(RefreshFailure::Reading)?;
        let online = reading::online_agents(
            self.authenticator.clone(),
            self.clock.clone(),
            &room_id,
            state,
            now,
        )
        .await
        .map_err(RefreshFailure::Reading)?;
        let network_agents = self
            .network_agent_ids(&view::agent_ids(&mutations, &online))
            .await;
        Ok(view::RoomReading {
            mutations,
            online,
            hidden,
            network_agents,
        })
    }

    /// 其中哪些是网络 Agent。答案不会变，问过的记着；查不了的这次先当普通 Agent，下次再问。
    async fn network_agent_ids(&self, candidates: &[AgentId]) -> HashSet<AgentId> {
        let unknown: Vec<AgentId> = {
            let kinds = self.kinds();
            candidates
                .iter()
                .filter(|id| !kinds.contains_key(id))
                .copied()
                .collect()
        };
        for batch in unknown.chunks(LOOKUP_BATCH) {
            match self.network_agents.network_agent_ids(batch).await {
                Ok(found) => {
                    let found: HashSet<AgentId> = found.into_iter().collect();
                    let mut kinds = self.kinds();
                    if kinds.len() + batch.len() > MAX_KNOWN_AGENT_KINDS {
                        kinds.clear();
                    }
                    for id in batch {
                        kinds.insert(*id, found.contains(id));
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        operation = error.operation(),
                        failure = ?error.kind(),
                        "围观公开大厅：查不了哪些是网络 Agent"
                    );
                    break;
                }
            }
        }
        let kinds = self.kinds();
        candidates
            .iter()
            .filter(|id| kinds.get(id) == Some(&true))
            .copied()
            .collect()
    }

    fn kinds(&self) -> MutexGuard<'_, HashMap<AgentId, bool>> {
        self.agent_kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// 目录里的一个公开大厅：围观只用得到这几样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchLobby {
    pub(crate) catalog_id: RoomCatalogId,
    pub(crate) name: String,
    pub(crate) slug: String,
}

fn find_lobby<'a>(lobbies: &'a [WatchLobby], slug: &str) -> Option<&'a WatchLobby> {
    lobbies.iter().find(|lobby| lobby.slug == slug).or_else(|| {
        if slug != DEFAULT_ALIAS {
            return None;
        }
        lobbies
            .iter()
            .find(|lobby| lobby.slug == DEFAULT_LOBBY_SLUG)
            .or_else(|| lobbies.first())
    })
}

#[derive(Default)]
struct DirectoryCache {
    /// 上次去查目录的时间，查失败也算。
    checked_at: Option<UtcMillis>,
    failed: bool,
    /// 上次查到的公开大厅；从没查到过时没有。
    lobbies: Option<Vec<WatchLobby>>,
}

impl DirectoryCache {
    fn due(&self, now: UtcMillis) -> bool {
        let wait = if self.failed {
            REFRESH_INTERVAL_MS
        } else {
            DIRECTORY_TTL_MS
        };
        self.checked_at
            .is_none_or(|checked_at| now.value().saturating_sub(checked_at.value()) >= wait)
    }
}

/// 一个公开大厅的快照，和“正在重读”的锁。
#[derive(Default)]
struct LobbySlot {
    refresh: tokio::sync::Mutex<()>,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    /// 快照读的时刻，和回给网页的 JSON。
    snapshot: Option<(UtcMillis, Bytes)>,
    /// 上次开始重读的时刻，读失败也算：失败后同样隔 3 秒再读，不追着 Synapse 问。
    attempted_at: Option<UtcMillis>,
}

impl LobbySlot {
    fn due(&self, now: UtcMillis) -> bool {
        self.state().attempted_at.is_none_or(|attempted_at| {
            now.value().saturating_sub(attempted_at.value()) >= REFRESH_INTERVAL_MS
        })
    }

    fn usable(&self, now: UtcMillis) -> Option<Bytes> {
        self.state()
            .snapshot
            .as_ref()
            .filter(|(taken_at, _)| now.value().saturating_sub(taken_at.value()) <= STALE_LIMIT_MS)
            .map(|(_, body)| body.clone())
    }

    fn attempted(&self, now: UtcMillis) {
        self.state().attempted_at = Some(now);
    }

    fn store(&self, taken_at: UtcMillis, body: Bytes) {
        self.state().snapshot = Some((taken_at, body));
    }

    fn state(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 一次重读没成的原因，只记日志。
enum RefreshFailure {
    Directory(RepositoryError),
    InvalidRoom,
    Matrix(MatrixFailure),
    Reading(reading::ReadingFailure),
    Encode,
}

impl RefreshFailure {
    fn log(&self, lobby: &str) {
        match self {
            Self::Directory(error) => tracing::warn!(
                lobby,
                operation = error.operation(),
                failure = ?error.kind(),
                "围观公开大厅：查不到大厅分片"
            ),
            Self::InvalidRoom => tracing::warn!(lobby, "围观公开大厅：大厅分片的房间号不对"),
            Self::Matrix(failure) => tracing::warn!(
                lobby,
                operation = ?failure.operation(),
                failure = ?failure.kind(),
                "围观公开大厅：Synapse 没读出来"
            ),
            Self::Reading(failure) => {
                tracing::warn!(lobby, failure = ?failure, "围观公开大厅：这次读到的验不了");
            }
            Self::Encode => tracing::warn!(lobby, "围观公开大厅：快照写不成 JSON"),
        }
    }
}
