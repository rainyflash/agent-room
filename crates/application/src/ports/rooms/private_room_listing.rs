//! 公开房间的设置（`specs/public-rooms/design.md`）。放在私人房间聚合之外：设置本身不改任何人的
//! 成员资格，只有房主能改。房间第一次公开时才有记录，之后改回私密也留着。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoom,
    room_listing::{RoomListingSettings, RoomShareCode},
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixResult, MatrixRoomId, PortFuture, PrivateRoomSnapshot, RoomRoleChange},
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

/// 一间子房间跟着父房间公开、改回私密时要看的几样（`specs/redesign/sub-rooms.md` 第 2d-5 步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomPrivacy {
    pub catalog_id: RoomCatalogId,
    pub matrix_room: MatrixRoomId,
    /// 现在有没有门。
    pub has_door: bool,
    /// 写过 `room.listing.v1`：第一次拆门，或者父房间公开时它没有门。只写第一次。
    pub listing_marked: bool,
    /// 目录上记的加不加密。
    pub end_to_end_encrypted: bool,
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

    /// 这几间房里正公开着的那些的分享码。本机 Agent 列能进的房间时给公开房间带上，主人贴过来的
    /// 分享链接按它认（`specs/public-rooms/design.md` 第 5 步）。默认一间一间问，库里一条语句查完。
    fn listed_share_codes<'a>(
        &'a self,
        catalog_ids: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<Vec<(RoomCatalogId, RoomShareCode)>>> {
        Box::pin(async move {
            let mut codes = Vec::new();
            for &catalog_id in catalog_ids {
                if let Some(listing) = self.listing(catalog_id).await?
                    && listing.settings.listed()
                {
                    codes.push((catalog_id, listing.share_code));
                }
            }
            Ok(codes)
        })
    }

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

    /// 新建时就公开的房间：在同一个事务里建房（目录上记成不加密）、写下设置，先锁住房主再查限额。
    /// 这个房间已经有了时回答 [`PrivateRoomListingSaveOutcome::Stale`]，交给调用方按幂等重试处理。
    fn create_listed_room<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        listing: &'a PrivateRoomListingRecord,
        created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>>;

    /// 存下转让了房主的房间。这间正公开着、接手的人（`room` 现在的房主）名下又有别的公开房间时不存，
    /// 回答 [`PrivateRoomListingSaveOutcome::OwnerLimitReached`]。和 [`Self::save_listing`] 一样在
    /// 事务里查，接手的人同时公开别的房间、原房主同时公开这一间，都只成一样。
    fn save_transferred_room<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>>;

    /// 和 [`Self::save_transferred_room`] 一样存，存下的同一个事务里记下原房主和接手的人的身份变动。
    /// 默认只存、不记：不关心记录的测试仓储不用管它。
    fn save_transferred_room_recording<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected: AggregateVersion,
        changed_at: UtcMillis,
        role_changes: &'a [RoomRoleChange],
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        let _ = role_changes;
        self.save_transferred_room(room, expected, changed_at)
    }

    /// 房间在 Matrix 上开了加密以后，把目录上的加密标记改成 true。
    fn mark_room_encrypted(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 这间房下面每一间子房间跟着它公开、改回私密要看的几样，按建的先后。归档了的也在：恢复以后
    /// 照样是这间房下面的。默认没有子房间：不关心子房间的测试仓储不用管它。
    fn sub_room_privacy(
        &self,
        parent_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomPrivacy>>> {
        let _ = parent_catalog_id;
        Box::pin(async { Ok(Vec::new()) })
    }

    /// 子房间在 Matrix 上写了 `room.listing.v1` 以后记下这一刻；记过的不动（只认第一次）。
    fn mark_sub_room_listed(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let _ = (catalog_id, at);
        Box::pin(async { Ok(()) })
    }
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
