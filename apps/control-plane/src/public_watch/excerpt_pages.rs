//! 分享的一段对话那一页（`/r/<分享码>/s/<段码>`，`specs/redesign/project-rooms.md`“分享一段对话”，
//! 3P-7a-2），不登录也打得开。
//!
//! - 每次打开都按分享码和段码再查一遍库（[`RoomExcerptPageLookup`]）：房间此刻还放着实况、段码是这间房的、
//!   第一句不早于这次开实况的那一刻。查到的（有或者没有）在内存里留 5 秒。
//! - 存着的只有事件 ID：以建房间的应用服务账号从最新往回读这间房，最多 500 条，读到第一句为止
//!   （[`ExcerptTimelineReader`]），经实况同一套解析和验签。撤回、隐藏的不显示，改过的显示改后的，每句开头
//!   280 个字，和实况一样。读到的样子在内存里留 60 秒，最多 500 段；同时读的最多 4 段，等不到算暂时打不开。
//! - 读不到第一句（这一段已经在 500 条以外），或者那几句都没了，是“看不了了”（410）；聊天服务器上这间房
//!   加了密，和没有这一段一样（404）。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use agent_room_application::{
    ports::{
        AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock,
        ExcerptPageRecord, MatrixEventId, NetworkAgentLookup, PortFuture, PublicLobbyMatrixReader,
        RoomAgentKind, RoomExcerptPageLookup, RoomPeopleDirectory,
    },
    room_people::shown_display_name,
};
use agent_room_bridge_core::messages::ProjectedMessageActor;
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    room_excerpts::RoomExcerptCode,
    room_listing::RoomShareCode,
    time::UtcMillis,
};

use super::{
    REFRESH_DEADLINE,
    agent_kinds::AgentKinds,
    excerpts::{ExcerptReading, ExcerptTimelineDependencies, ExcerptTimelineReader, ReadBack},
    live_view::LIVE_TEXT_CHARACTERS,
    view::{self, Shown},
};

/// 按码查到的在内存里留这么久。
const LOOKUP_TTL_MS: i64 = 5_000;
/// 记着的码最多这么多个，满了先清掉过期的，还满就全清。
const MAX_LOOKUPS: usize = 4_096;
/// 读到的样子留这么久。
const VIEW_TTL_MS: i64 = 60_000;
/// 最多留这么多段读到的样子。
const MAX_VIEWS: usize = 500;
/// 最多同时读这么多段。
const MAX_CONCURRENT_READS: usize = 4;

pub(crate) struct RoomExcerptPagesDependencies {
    /// 按分享码和段码找一段（库里）。
    pub(crate) lookup: Arc<dyn RoomExcerptPageLookup>,
    /// 说话的本机 Agent 是谁的。
    pub(crate) people: Arc<dyn RoomPeopleDirectory>,
    pub(crate) network_agents: Arc<dyn NetworkAgentLookup>,
    pub(crate) reader: Arc<dyn PublicLobbyMatrixReader>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExcerptPageFailure {
    /// 房间不放实况、没有这个码、这一段早于这次开实况、聊天服务器上加了密：不分是哪一种。
    NotFound,
    /// 库或者聊天服务器暂时不通，或者同时读的太多。
    Unavailable,
}

/// 打开这一段看到的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExcerptPage {
    Shown(ShownExcerpt),
    /// 读不到第一句了，或者那几句都没了。
    Gone {
        room_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShownExcerpt {
    pub(crate) room_name: String,
    /// 看得见的那几句，按时间先后，至少一句。
    pub(crate) lines: Vec<ExcerptLine>,
}

/// 一句话。名字、正文都是别人写的，放进页面前要转义。
// 几个互不相干的标记平铺着最好读。
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExcerptLine {
    pub(crate) name: String,
    pub(crate) author: ExcerptAuthor,
    /// 开头最多 280 个字；不公开的消息是空的。
    pub(crate) text: String,
    pub(crate) truncated: bool,
    /// 敏感或受限的消息：只说“一条不公开的消息”。
    pub(crate) withheld: bool,
    /// 带了文件（只说有）。
    pub(crate) attachment: bool,
    pub(crate) edited: bool,
    pub(crate) sent_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExcerptAuthor {
    Person,
    /// 本机 Agent，带主人给别人看的名字（问不到的为空）。
    Agent {
        owner: Option<String>,
    },
    NetworkAgent,
}

/// 分享页那一边用到的：打开一段。
pub(crate) trait RoomExcerptPageSource: Send + Sync {
    fn page<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, Result<Arc<ExcerptPage>, ExcerptPageFailure>>;
}

pub(crate) struct RoomExcerptPages {
    lookup: Arc<dyn RoomExcerptPageLookup>,
    people: Arc<dyn RoomPeopleDirectory>,
    timeline: ExcerptTimelineReader,
    network_agents: AgentKinds,
    clock: Arc<dyn Clock>,
    lookups: Mutex<HashMap<(RoomShareCode, RoomExcerptCode), Looked>>,
    views: Mutex<HashMap<ViewKey, Kept>>,
    reads: tokio::sync::Semaphore,
}

struct Looked {
    checked_at: UtcMillis,
    record: Option<ExcerptPageRecord>,
}

/// 读到的样子按这一段和这次开实况的那一刻存：再开实况是新的一刻。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ViewKey {
    code: RoomExcerptCode,
    enabled_at: i64,
}

struct Kept {
    stored_at: UtcMillis,
    page: Arc<ExcerptPage>,
}

impl RoomExcerptPages {
    pub(crate) fn new(dependencies: RoomExcerptPagesDependencies) -> Self {
        Self {
            lookup: dependencies.lookup,
            people: dependencies.people,
            timeline: ExcerptTimelineReader::new(ExcerptTimelineDependencies {
                reader: dependencies.reader,
                verification: dependencies.verification,
                signatures: dependencies.signatures,
                clock: dependencies.clock.clone(),
            }),
            network_agents: AgentKinds::new(dependencies.network_agents),
            clock: dependencies.clock,
            lookups: Mutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
            reads: tokio::sync::Semaphore::new(MAX_CONCURRENT_READS),
        }
    }

    async fn page_of(
        &self,
        share_code: &RoomShareCode,
        code: &RoomExcerptCode,
    ) -> Result<Arc<ExcerptPage>, ExcerptPageFailure> {
        let record = self
            .record(share_code, code)
            .await?
            .ok_or(ExcerptPageFailure::NotFound)?;
        let key = ViewKey {
            code: code.clone(),
            enabled_at: record.enabled_at.value(),
        };
        if let Some(page) = self.kept(&key) {
            return Ok(page);
        }
        let Ok(Ok(_permit)) = tokio::time::timeout(REFRESH_DEADLINE, self.reads.acquire()).await
        else {
            return Err(ExcerptPageFailure::Unavailable);
        };
        // 排队的时候别的请求可能已经读完了。
        if let Some(page) = self.kept(&key) {
            return Ok(page);
        }
        let page = Arc::new(self.read(&record).await?);
        self.keep(key, page.clone());
        Ok(page)
    }

    /// 库里的这一段。5 秒内查过就用上次的。
    async fn record(
        &self,
        share_code: &RoomShareCode,
        code: &RoomExcerptCode,
    ) -> Result<Option<ExcerptPageRecord>, ExcerptPageFailure> {
        let now = self.clock.now();
        let key = (share_code.clone(), code.clone());
        if let Some(entry) = self.lookups().get(&key)
            && within(entry.checked_at, now, LOOKUP_TTL_MS)
        {
            return Ok(entry.record.clone());
        }
        let record = self
            .lookup
            .excerpt_page(share_code, code)
            .await
            .map_err(|error| {
                tracing::warn!(
                    operation = error.operation(),
                    failure = ?error.kind(),
                    "分享的一段对话：库里查不了"
                );
                ExcerptPageFailure::Unavailable
            })?;
        let mut lookups = self.lookups();
        if lookups.len() >= MAX_LOOKUPS {
            lookups.retain(|_, entry| within(entry.checked_at, now, LOOKUP_TTL_MS));
            if lookups.len() >= MAX_LOOKUPS {
                lookups.clear();
            }
        }
        lookups.insert(
            key,
            Looked {
                checked_at: now,
                record: record.clone(),
            },
        );
        Ok(record)
    }

    async fn read(&self, record: &ExcerptPageRecord) -> Result<ExcerptPage, ExcerptPageFailure> {
        let gone = || ExcerptPage::Gone {
            room_name: record.room_name.clone(),
        };
        let Some(first) = record.event_ids.first() else {
            return Ok(gone());
        };
        match self.timeline.read_back(&record.matrix_room_id, first).await {
            Ok(ReadBack::Found(reading)) => Ok(self
                .assemble(record, &reading)
                .await
                .map_or_else(gone, ExcerptPage::Shown)),
            Ok(ReadBack::NotReachable) => Ok(gone()),
            Ok(ReadBack::Encrypted) => {
                tracing::warn!("分享的一段对话：目录记着不加密，聊天服务器上却加了密，不放");
                Err(ExcerptPageFailure::NotFound)
            }
            Err(failure) => {
                failure.log();
                Err(ExcerptPageFailure::Unavailable)
            }
        }
    }

    /// 读到的里面属于这一段、此刻还看得见的那几句；一句都没有是 `None`。
    async fn assemble(
        &self,
        record: &ExcerptPageRecord,
        reading: &ExcerptReading,
    ) -> Option<ShownExcerpt> {
        let wanted: HashSet<&str> = record.event_ids.iter().map(MatrixEventId::as_str).collect();
        let shown: Vec<Shown<'_>> = view::all_visible_messages(&reading.mutations, &reading.hidden)
            .into_iter()
            .filter(|message| {
                wanted.contains(message.original.event_id.as_str())
                    && view::sent_at(message) >= record.enabled_at.value()
            })
            .collect();
        if shown.is_empty() {
            return None;
        }
        let mut authors: Vec<AgentId> = shown
            .iter()
            .filter_map(|message| Some(message.original.actor.agent_identity()?.agent_id()))
            .collect();
        authors.sort_unstable();
        authors.dedup();
        let network_agents = self.network_agents.network_agent_ids(&authors).await;
        let owners = self
            .owners(record.catalog_id, &authors, &network_agents)
            .await;
        Some(ShownExcerpt {
            room_name: record.room_name.clone(),
            lines: shown
                .iter()
                .map(|message| line(message, &network_agents, &owners))
                .collect(),
        })
    }

    /// 说话的本机 Agent 各是谁的。问不了的这次不写主人。
    async fn owners(
        &self,
        catalog_id: RoomCatalogId,
        authors: &[AgentId],
        network_agents: &HashSet<AgentId>,
    ) -> HashMap<AgentId, String> {
        let local: Vec<AgentId> = authors
            .iter()
            .filter(|id| !network_agents.contains(id))
            .copied()
            .collect();
        if local.is_empty() {
            return HashMap::new();
        }
        match self.people.agent_owners(catalog_id, &local).await {
            Ok(records) => records
                .into_iter()
                .filter_map(|record| match record.kind {
                    RoomAgentKind::Local => record
                        .owner
                        .map(|owner| (record.agent_id, shown_display_name(&owner.display_name))),
                    RoomAgentKind::Network => None,
                })
                .collect(),
            Err(error) => {
                tracing::warn!(
                    operation = error.operation(),
                    failure = ?error.kind(),
                    "分享的一段对话：查不了 Agent 是谁的"
                );
                HashMap::new()
            }
        }
    }

    fn kept(&self, key: &ViewKey) -> Option<Arc<ExcerptPage>> {
        let now = self.clock.now();
        self.views()
            .get(key)
            .filter(|kept| within(kept.stored_at, now, VIEW_TTL_MS))
            .map(|kept| kept.page.clone())
    }

    fn keep(&self, key: ViewKey, page: Arc<ExcerptPage>) {
        let now = self.clock.now();
        let mut views = self.views();
        if views.len() >= MAX_VIEWS {
            views.retain(|_, kept| within(kept.stored_at, now, VIEW_TTL_MS));
            if views.len() >= MAX_VIEWS {
                views.clear();
            }
        }
        views.insert(
            key,
            Kept {
                stored_at: now,
                page,
            },
        );
    }

    fn lookups(&self) -> MutexGuard<'_, HashMap<(RoomShareCode, RoomExcerptCode), Looked>> {
        self.lookups.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn views(&self) -> MutexGuard<'_, HashMap<ViewKey, Kept>> {
        self.views.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RoomExcerptPageSource for RoomExcerptPages {
    fn page<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, Result<Arc<ExcerptPage>, ExcerptPageFailure>> {
        Box::pin(self.page_of(share_code, code))
    }
}

fn line(
    message: &Shown<'_>,
    network_agents: &HashSet<AgentId>,
    owners: &HashMap<AgentId, String>,
) -> ExcerptLine {
    let (name, author) = match &message.original.actor {
        ProjectedMessageActor::Agent { identity, .. } => {
            let agent_id = identity.agent_id();
            let author = if network_agents.contains(&agent_id) {
                ExcerptAuthor::NetworkAgent
            } else {
                ExcerptAuthor::Agent {
                    owner: owners.get(&agent_id).cloned(),
                }
            };
            (identity.display_name().to_owned(), author)
        }
        ProjectedMessageActor::Human { display_name, .. } => {
            (shown_display_name(display_name), ExcerptAuthor::Person)
        }
    };
    let body = view::message_body(message, LIVE_TEXT_CHARACTERS);
    ExcerptLine {
        name,
        author,
        text: body.text,
        truncated: body.truncated,
        withheld: body.withheld,
        attachment: body.attachment,
        edited: body.edited,
        sent_at_unix_ms: body.sent_at_unix_ms,
    }
}

fn within(at: UtcMillis, now: UtcMillis, ttl_ms: i64) -> bool {
    now.value().saturating_sub(at.value()) < ttl_ms
}
