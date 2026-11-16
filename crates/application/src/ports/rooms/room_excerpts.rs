//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，表 `room_excerpt`）：存的是哪几句
//! （Matrix 事件 ID）、哪间房、第一句的时间、谁分享的、什么时候，不存话。分享的时候要读一遍房间，
//! 看这几句在不在、连不连着、有几句：读房间的是控制面（[`RoomExcerptTimeline`]），和分享页的实况
//! 同一套解析。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_excerpts::RoomExcerptCode,
    room_listing::RoomShareCode,
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixEventId, MatrixRoomId, PortFuture},
};

/// 存着的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomExcerptRecord {
    pub code: RoomExcerptCode,
    pub catalog_id: RoomCatalogId,
    /// 那几句的事件 ID，按时间先后，最多 30 个。
    pub event_ids: Vec<MatrixEventId>,
    /// 第一句的时间：过了房间的保留期就删。
    pub first_event_at: UtcMillis,
    pub shared_by: PrincipalId,
    pub shared_at: UtcMillis,
}

/// 此刻放着实况、能分享一段的房间（和分享页 `/r/<码>/live` 认的是同几样）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptRoomRecord {
    pub matrix_room_id: MatrixRoomId,
    /// 开实况的那一刻：只认这以后说的话。
    pub enabled_at: UtcMillis,
    pub share_code: RoomShareCode,
}

/// 记下一段的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertRoomExcerptOutcome {
    Inserted,
    /// 段码撞上了别的：换一个再存。
    CodeTaken,
    /// 存的那一下实况已经关了，或者换了一刻。
    NotLive,
}

/// 一间房里存着的段，和这间房此刻的分享码（拼链接用；没有分享码的房间是空的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomExcerptListing {
    pub share_code: Option<RoomShareCode>,
    /// 新分享的在前。
    pub excerpts: Vec<RoomExcerptRecord>,
}

pub trait RoomExcerptStore: Send + Sync {
    /// 放着实况的这间房：公开着、直接进、目录上不加密、在用、没被平台拿掉、不是子房间、开着实况。
    /// 别的都是空的，不分是哪一种。
    fn excerpt_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<ExcerptRoomRecord>>>;

    /// 记下一段。存的时候锁着这间房的实况那一行，还是 `enabled_at` 那一刻开着才记；和关实况撞上时，
    /// 要么这里等它关完、记不下，要么它等这里存完、连这一段一起删。
    fn insert_excerpt<'a>(
        &'a self,
        excerpt: &'a RoomExcerptRecord,
        enabled_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<InsertRoomExcerptOutcome>>;

    /// 这间房里存着的段；`shared_by` 给了就只列他分享的。最多列最近的 200 段。
    fn room_excerpts(
        &self,
        catalog_id: RoomCatalogId,
        shared_by: Option<PrincipalId>,
    ) -> PortFuture<'_, RepositoryResult<RoomExcerptListing>>;

    fn find_excerpt<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomExcerptRecord>>>;

    /// 删掉一段，回删没删到。
    fn delete_excerpt<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    /// 每天一次：删掉第一句过了房间保留期（没设的按 `default_retention_days`）的段，和房间此刻没开着
    /// 实况、或者第一句早于开实况那一刻的段。回删了几段。
    fn prune_excerpts(
        &self,
        now: UtcMillis,
        default_retention_days: u16,
    ) -> PortFuture<'_, RepositoryResult<u64>>;
}

/// 读房间时看到的一句：事件 ID 和服务器收到它的时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptMessageRef {
    pub event_id: MatrixEventId,
    pub sent_at: UtcMillis,
}

/// 从最新往回读到某一句为止看到的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcerptTimelineRead {
    /// 读到了那一句。里面是读到的每一句看得见的话（改过的照改后的，撤回、隐藏的不在里面），按时间先后。
    Found(Vec<ExcerptMessageRef>),
    /// 最近 500 条事件里没有那一句：太早了，或者根本不是这间房的。
    NotReachable,
    /// 聊天服务器上这间房加了密：不放。
    Encrypted,
}

/// 聊天服务器暂时读不到，或者读到的验不了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcerptTimelineUnavailable;

/// 以应用服务从最新往回读这间房（最多 500 条事件），读到 `first_event_id` 为止，经分享页实况同一套解析
/// 和验签。控制面实现。
pub trait RoomExcerptTimeline: Send + Sync {
    fn visible_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        first_event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<ExcerptTimelineRead, ExcerptTimelineUnavailable>>;
}

/// 打开分享的一段时（`/r/<分享码>/s/<段码>`）要的：这间房此刻还放着实况、码对得上、第一句不早于这次开
/// 实况的那一刻。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptPageRecord {
    pub catalog_id: RoomCatalogId,
    pub room_name: String,
    pub matrix_room_id: MatrixRoomId,
    pub enabled_at: UtcMillis,
    /// 那几句的事件 ID，按时间先后。
    pub event_ids: Vec<MatrixEventId>,
    pub first_event_at: UtcMillis,
}

/// 按分享码和段码找一段，不登录也打得开的那一页用。
pub trait RoomExcerptPageLookup: Send + Sync {
    /// 房间不放实况、没有这个分享码或段码、段码是别的房间的、这一段早于这次开实况，都是空的，不分是哪
    /// 一种。
    fn excerpt_page<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, RepositoryResult<Option<ExcerptPageRecord>>>;
}
