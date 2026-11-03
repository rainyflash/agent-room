//! 发现页的目录（`specs/public-rooms/design.md`）：公开着、还在用的房间，谁都看得到名字、简介、房主、
//! 人数这些，房间里说的话和成员名单都不给。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixResult, MatrixRoomId, PortFuture},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRoomSort {
    /// 最近有人说话的在前，没人说过话的排在最后。
    Active,
    /// 人多的在前。
    Members,
    /// 最近才公开的在前。
    Newest,
}

/// 目录里找房间。`text` 是去掉首尾空白、不空的原样文字，由存储那边转义成模糊匹配。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomSearch {
    pub text: Option<String>,
    pub join_mode: Option<RoomJoinMode>,
    /// 只要这个时间以后有人说过话的。
    pub active_since: Option<UtcMillis>,
    pub unencrypted_only: bool,
    pub sort: PublicRoomSort,
    pub offset: u32,
    pub limit: u32,
}

/// 目录里的一间房。房主的昵称是存着的原样，给人看之前再处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomRecord {
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub description: String,
    pub owner_display_name: String,
    /// 已加入的人。
    pub member_count: u32,
    /// 此刻在房间里的 Agent：本机的有确认了的位置，网络的记着在这间房。
    pub agent_count: u32,
    pub join_mode: RoomJoinMode,
    pub end_to_end_encrypted: bool,
    pub share_code: RoomShareCode,
    pub created_at: UtcMillis,
    pub last_message_at: Option<UtcMillis>,
}

/// 一间房的详情：比目录多几位管理员（不含房主），先进房间的在前。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomDetailRecord {
    pub room: PublicRoomRecord,
    pub admin_display_names: Vec<String>,
}

/// 登录的人在这间房的处境，发现页按它给按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRoomViewerState {
    Joined,
    Invited,
    /// 申请了，在等批准。
    Pending,
    Banned,
}

pub trait PublicRoomDirectoryStore: Send + Sync {
    /// 公开着、还在用的房间，按 `search` 筛、排、翻页。
    fn search_public_rooms<'a>(
        &'a self,
        search: &'a PublicRoomSearch,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicRoomRecord>>>;

    /// 分享码对应的房间；不公开、关了、没有这个码都是 `None`。
    fn public_room_by_share_code<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, RepositoryResult<Option<PublicRoomDetailRecord>>>;

    /// 他在这几间房的处境，不在、离开了、拒绝了的不列。
    fn public_room_viewer_states<'a>(
        &'a self,
        viewer: PrincipalId,
        rooms: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<Vec<(RoomCatalogId, PublicRoomViewerState)>>>;
}

/// 发现页的“最近有人说话”：控制面定时替每间公开房间记下来。
pub trait PublicRoomActivityStore: Send + Sync {
    /// 公开着、还在用的房间和它们在 Matrix 上的房间。
    fn listed_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>>;

    /// 记下这间房最近有人说话的时间，只往后挪。
    fn record_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;
}

pub trait PublicRoomActivityMatrix: Send + Sync {
    /// 以应用服务读这间房最新一条消息事件的时间。只看时间、不读内容，加密的房间也一样；
    /// 一条都没有时回 `None`。
    fn latest_message_at<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<UtcMillis>>>;
}
