//! 子房间的归档和恢复（`specs/redesign/sub-rooms.md` 的“归档和恢复”，第 2e-2 步）存的时候要的。
//!
//! 归没归档还是记在房间自己的状态上；什么时候、谁、为什么记在父子关系那一行（`archived_*` 三列），
//! 和状态在同一个事务里写、恢复时一起清。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoom,
    sub_room_archive::SubRoomArchiveReason,
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

/// 一间子房间归档的记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomArchiveRecord {
    pub at: UtcMillis,
    /// 谁归档的。自动归档的、跟着父房间归档的没有。
    pub by: Option<PrincipalId>,
    pub reason: SubRoomArchiveReason,
}

/// 归档一间子房间要存的。
#[derive(Debug, Clone, Copy)]
pub struct SubRoomArchival<'a> {
    /// 已经改成归档的子房间。
    pub room: &'a PrivateRoom,
    /// 改之前读到的版本。
    pub expected_version: AggregateVersion,
    pub parent_catalog_id: RoomCatalogId,
    /// 谁归档、为什么。跟着父房间归档不走这里（在存父房间的事务里一起做）。
    pub by: Option<PrincipalId>,
    pub reason: SubRoomArchiveReason,
    pub at: UtcMillis,
}

/// 恢复一间子房间要存的。
#[derive(Debug, Clone, Copy)]
pub struct SubRoomRestoration<'a> {
    /// 已经改回在用的子房间。
    pub room: &'a PrivateRoom,
    /// 改之前读到的版本。
    pub expected_version: AggregateVersion,
    pub parent_catalog_id: RoomCatalogId,
    pub by: PrincipalId,
    pub at: UtcMillis,
}

/// 归档、恢复存的时候怎么样了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomArchiveOutcome {
    Done,
    /// 父房间不在用了、这间子房间不在了或者不挂在这间房下面；恢复时还有：它不是归档的、是跟着父房间
    /// 归档的。
    Unavailable,
    /// 恢复时：这间房里另一间没归档的子房间已经叫这个名字（不分大小写）。
    NameTaken,
    /// 恢复时：这间房里没归档的子房间已经满了。
    LimitReached,
}

/// 子房间的归档和恢复也是子房间存储口子的一部分（`SubRoomStore` 的上层）。
pub trait SubRoomArchiveStore: Send + Sync {
    /// 归档：房间的状态、父子关系上的记录、这一间在等的申请作废、聊天服务器上锁住它的事排上、审计，
    /// 在同一个事务里，先锁父房间在目录里的那一行（和建子房间、改名排着队）。父房间不在用、这间不挂
    /// 在它下面时什么都不写。读到以后房间又被存过的报 `Conflict`。
    fn archive_sub_room<'a>(
        &'a self,
        archival: &'a SubRoomArchival<'a>,
    ) -> PortFuture<'a, RepositoryResult<SubRoomArchiveOutcome>>;

    /// 恢复：房间的状态、清掉父子关系上的记录、聊天服务器上解开它的事排上、审计，在同一个事务里。和
    /// 建子房间一样先锁父房间在目录里的那一行，再数重名和个数（不算这一间自己）；撞了名字、满了时
    /// 什么都不写。读到以后房间又被存过的报 `Conflict`。
    fn restore_sub_room<'a>(
        &'a self,
        restoration: &'a SubRoomRestoration<'a>,
    ) -> PortFuture<'a, RepositoryResult<SubRoomArchiveOutcome>>;
}
