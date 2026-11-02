//! 公开房间的设置（`specs/public-rooms/design.md`）。放在私人房间聚合之外：设置本身不改任何人的
//! 成员资格，只有房主能改。房间第一次公开时才有记录，之后改回私密也留着。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomListingSettings, RoomShareCode},
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixResult, MatrixRoomId, PortFuture},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomListingRecord {
    pub catalog_id: RoomCatalogId,
    pub settings: RoomListingSettings,
    pub share_code: RoomShareCode,
    /// 第一次公开的时间，之后不变。Matrix 上的 `room.listing.v1` 写的也是它。
    pub first_listed_at: UtcMillis,
    pub updated_by: PrincipalId,
    pub updated_at: UtcMillis,
    pub version: AggregateVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomListingSaveOutcome {
    Saved,
    /// 房主名下已经有一间别的公开房间（每个账户最多 1 间）。
    OwnerLimitReached,
    /// 新生成的分享码被别的房间占了，换一个再存。
    ShareCodeTaken,
    /// 别人刚改过，版本对不上。
    Stale,
}

pub trait PrivateRoomListingStore: Send + Sync {
    fn listing(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomListingRecord>>>;

    /// 房主名下还在使用中、正公开着的房间，供公开之前先查一遍限额。
    fn listed_rooms_owned_by(
        &self,
        owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>>;

    /// 写下新的设置。`expected` 为空表示这间房还没有记录。公开时在同一个事务里再查一遍房主名下
    /// 别的公开房间，两个请求同时公开两间也只成一间。
    fn save_listing<'a>(
        &'a self,
        record: &'a PrivateRoomListingRecord,
        expected: Option<AggregateVersion>,
        owner: PrincipalId,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>>;

    /// 房间在 Matrix 上开了加密以后，把目录上的加密标记改成 true。
    fn mark_room_encrypted(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>>;
}

/// 公开设置在 Matrix 上的两样副作用，都以应用服务写房间状态。
pub trait PrivateRoomListingMatrix: Send + Sync {
    /// 写 `io.github.rainyflash.agentroom.room.listing.v1`：房间第一次公开的时间。之后进来的人
    /// 别人不给他们重发房间钥匙，他们自己也不去要（第 2 步，`room_listing`）。
    fn mark_listed<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        first_listed_at: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<()>>;

    /// 写 `m.room.encryption`。不加密的公开房间改回私密时打开加密，之后再也关不掉。
    fn enable_encryption<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>>;
}
