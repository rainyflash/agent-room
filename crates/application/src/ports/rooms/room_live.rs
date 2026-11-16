//! 分享页放实况（`specs/redesign/project-rooms.md`“分享页放实况”）：开没开、什么时候开的，房间此刻
//! 满不满足放实况的那几样。表是 `room_live_view`，关了就删。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::RoomJoinMode,
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixResult, MatrixRoomId, PortFuture},
};

/// 开着的实况：谁、什么时候开的。只放这一刻以后说的话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomLiveViewRecord {
    pub enabled_by: PrincipalId,
    pub enabled_at: UtcMillis,
}

/// 库里记着的、和放不放实况有关的几样。房间在不在使用中、是不是子房间看房间快照，不在这里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomLiveViewFacts {
    /// 公开设置：从没公开过的房间没有。
    pub listing: Option<(bool, RoomJoinMode)>,
    /// 目录上记着的加没加密（`room_catalog_entry.end_to_end_encrypted`）。
    pub end_to_end_encrypted: bool,
    /// 平台冻结了，或者从目录里拿掉了。
    pub platform_hold: bool,
    pub live_view: Option<RoomLiveViewRecord>,
}

/// 打开实况的结果。库里在同一句里再看一遍那几样（锁着房间目录和公开设置那两行），和改设置、
/// 归档撞上时以库里的为准。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnableRoomLiveViewOutcome {
    Enabled(RoomLiveViewRecord),
    /// 本来就开着：照旧是原来那一刻。
    AlreadyEnabled(RoomLiveViewRecord),
    /// 存的那一下房间已经不满足了（刚改了设置、刚归档）。
    NotEligible,
}

pub trait RoomLiveViewStore: Send + Sync {
    /// 这间房和实况有关的几样。没有这个房间时为空。
    fn live_view_facts(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomLiveViewFacts>>>;

    /// 打开实况，`at` 是开的那一刻。库里再看一遍：公开着、在使用中、不加密、直接进、没被平台拿掉、
    /// 不是子房间，才记下。
    fn enable_live_view(
        &self,
        catalog_id: RoomCatalogId,
        enabled_by: PrincipalId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<EnableRoomLiveViewOutcome>>;

    /// 关掉实况，有就删。
    fn disable_live_view(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RepositoryResult<()>>;
}

/// 问聊天服务器：房间状态里有没有 `m.room.encryption`。目录上的标记之外再看一遍，两样都不加密才放。
pub trait RoomEncryptionState: Send + Sync {
    fn room_encrypted<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<bool>>;
}
