//! 子房间最近有人说话的时间和 30 天没人说话自动归档（`specs/redesign/sub-rooms.md` 的“归档和恢复”，
//! 第 2j 步）存的时候要的。和发现页用同一套读法（[`super::PublicRoomActivityMatrix`]）：以应用服务读
//! 最新一条消息事件的时间，只要时间、不读内容，写进父子关系那一行的 `last_message_at`。“浏览子房间”
//! 上的最近说话和自动归档都按它。

use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixRoomId, PortFuture},
};

/// 每小时那一轮领走的一间安静的子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietSubRoom {
    pub catalog_id: RoomCatalogId,
    pub parent_catalog_id: RoomCatalogId,
    pub matrix_room: MatrixRoomId,
}

pub trait SubRoomActivityStore: Send + Sync {
    /// 没归档、父房间也还在用的子房间和它们在 Matrix 上的房间。
    fn sub_rooms_for_activity(
        &self,
    ) -> PortFuture<'_, RepositoryResult<Vec<(RoomCatalogId, MatrixRoomId)>>>;

    /// 记下这间子房间最近有人说话的时间，只往后挪。不算改子房间：不动房间的版本。
    fn record_sub_room_last_message(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 自动归档（2j-2）：领走最多 `limit` 间、租到 `lease_until`。领的是父房间开着自动归档（没定过三项
    /// 设置的照默认开着）、两间房都在用、`quiet_before` 以前就没人说话的子房间：最近说话、最近恢复、
    /// 建的时间里最晚的那个早于它。别人领走、租期还没到 `now` 的不领。
    fn claim_quiet_sub_rooms(
        &self,
        quiet_before: UtcMillis,
        now: UtcMillis,
        lease_until: UtcMillis,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<QuietSubRoom>>>;
}
