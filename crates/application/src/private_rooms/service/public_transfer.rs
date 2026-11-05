//! 转让房主也守着每个账户 1 间公开房间（`specs/public-rooms/design.md` 的“限额”）。私密的房间照常转：
//! 接手的人以后要公开，公开的时候再查。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoom,
    version::AggregateVersion,
};

use crate::ports::PrivateRoomListingSaveOutcome;

use super::{
    super::{
        PrivateRoomFailureStage, PrivateRoomResult,
        failure::{failure, repository},
    },
    PrivateRoomFailureKind, PrivateRoomService,
    public_creation::limit_reached,
};

impl PrivateRoomService {
    /// 先查一遍，免得白改一次 Matrix 上的发言权；存的时候在事务里还会再查。
    pub(super) async fn ensure_transfer_quota(
        &self,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let listing =
            self.listings.listing(catalog_id).await.map_err(|error| {
                repository(operation, PrivateRoomFailureStage::Directory, &error)
            })?;
        if !listing.is_some_and(|listing| listing.settings.listed()) {
            return Ok(());
        }
        let listed = self
            .listings
            .listed_rooms_owned_by(target)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?;
        if listed.iter().any(|room| *room != catalog_id) {
            Err(limit_reached(operation))
        } else {
            Ok(())
        }
    }

    /// 存下转让。返回 `false` 表示事务里才发现接手的人刚公开了别的房间，什么都没存。
    pub(super) async fn save_transferred_room(
        &self,
        room: &PrivateRoom,
        expected: AggregateVersion,
        operation: &'static str,
    ) -> PrivateRoomResult<bool> {
        let outcome = self
            .listings
            .save_transferred_room(room, expected, self.clock.now())
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?;
        match outcome {
            PrivateRoomListingSaveOutcome::Saved => Ok(true),
            PrivateRoomListingSaveOutcome::OwnerLimitReached => Ok(false),
            PrivateRoomListingSaveOutcome::ShareCodeTaken
            | PrivateRoomListingSaveOutcome::Stale => Err(failure(
                operation,
                PrivateRoomFailureStage::Persistence,
                PrivateRoomFailureKind::Conflict,
            )),
        }
    }
}
