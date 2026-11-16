//! 子房间最近有人说话的时间（`specs/redesign/sub-rooms.md` 的“归档和恢复”，第 2j 步）存的时候要的。
//! 和发现页用同一套读法（[`super::PublicRoomActivityMatrix`]）：以应用服务读最新一条消息事件的时间，
//! 只要时间、不读内容，写进父子关系那一行的 `last_message_at`。“浏览子房间”上的最近说话和 30 天没人
//! 说话自动归档都按它。

use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixRoomId, PortFuture},
};

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
}
