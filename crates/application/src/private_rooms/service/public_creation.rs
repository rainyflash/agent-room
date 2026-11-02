//! 新建时就选公开（`specs/public-rooms/design.md` 第 3a-2 步）：房间不加密，建好就公开。Matrix 上照旧
//! 隐藏、邀请制，只是没写 `m.room.encryption`；建好以后先写下公开的时间（`room.listing.v1`），再在
//! 一个事务里存房间和公开设置，顺带查每个账户只能有一间公开房间。

use agent_room_domain::{
    room_listing::{RoomListingSettings, RoomShareCode},
    version::AggregateVersion,
};

use crate::{
    ports::{
        MatrixRoomId, PrivateRoomListingRecord, PrivateRoomListingSaveOutcome, PrivateRoomSnapshot,
    },
    private_rooms::listing::{SHARE_CODE_ATTEMPTS, random_share_code},
};

use super::{
    super::{
        CreatePrivateRoom, PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage,
        PrivateRoomResult,
        failure::{domain, failure, matrix, repository},
    },
    PrivateRoomService, ensure_same_creation,
};

impl PrivateRoomService {
    /// 同一个房间号已经建过：名字、说明这些要一样，公开不公开也要一样，不然是另一个请求撞了号。
    pub(super) async fn ensure_same_room(
        &self,
        request: &CreatePrivateRoom,
        existing: PrivateRoomSnapshot,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let existing = ensure_same_creation(request, existing, operation)?;
        let listing = self
            .listings
            .listing(request.catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?;
        if listing.is_some() == request.listing.is_some() {
            Ok(existing)
        } else {
            Err(conflict(operation))
        }
    }

    /// 存的时候撞上已经有的房间：多半是同一个请求的重试抢先存好了。
    pub(super) async fn ensure_created_by_retry(
        &self,
        request: &CreatePrivateRoom,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let existing = self
            .find(request.catalog_id, operation)
            .await?
            .ok_or_else(|| conflict(operation))?;
        self.ensure_same_room(request, existing, operation).await
    }

    /// 先查一遍限额，免得白建一个 Matrix 房间；存的时候在事务里还会再查。
    pub(super) async fn ensure_public_room_quota(
        &self,
        request: &CreatePrivateRoom,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let listed = self
            .listings
            .listed_rooms_owned_by(request.actor.principal_id())
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?;
        if listed.is_empty() {
            Ok(())
        } else {
            Err(limit_reached(operation))
        }
    }

    /// Matrix 房间建好以后：写下公开的时间，再把房间和公开设置一起存下。分享码撞了换一个。
    pub(super) async fn save_public_room(
        &self,
        request: &CreatePrivateRoom,
        settings: RoomListingSettings,
        snapshot: PrivateRoomSnapshot,
        matrix_room_id: &MatrixRoomId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let now = self.clock.now();
        // 和之后才公开的房间一样写下时间：以后改回私密又公开，这个时间以前的房间钥匙照旧不给。
        self.listing_matrix
            .mark_listed(matrix_room_id, now)
            .await
            .map_err(|error| matrix(operation, error))?;
        let mut record = PrivateRoomListingRecord {
            catalog_id: request.catalog_id,
            settings,
            share_code: self.public_share_code(operation)?,
            first_listed_at: now,
            updated_by: request.actor.principal_id(),
            updated_at: now,
            version: AggregateVersion::INITIAL
                .next()
                .map_err(|error| domain(operation, &error))?,
        };
        for _ in 0..SHARE_CODE_ATTEMPTS {
            let outcome = self
                .listings
                .create_listed_room(&snapshot, &record, now)
                .await
                .map_err(|error| {
                    repository(operation, PrivateRoomFailureStage::Persistence, &error)
                })?;
            match outcome {
                PrivateRoomListingSaveOutcome::Saved => return Ok(snapshot),
                PrivateRoomListingSaveOutcome::ShareCodeTaken => {
                    record.share_code = self.public_share_code(operation)?;
                }
                PrivateRoomListingSaveOutcome::OwnerLimitReached => {
                    return Err(limit_reached(operation));
                }
                PrivateRoomListingSaveOutcome::Stale => {
                    return self.ensure_created_by_retry(request, operation).await;
                }
            }
        }
        Err(failure(
            operation,
            PrivateRoomFailureStage::Persistence,
            PrivateRoomFailureKind::Internal,
        ))
    }

    fn public_share_code(&self, operation: &'static str) -> PrivateRoomResult<RoomShareCode> {
        random_share_code(self.secrets.as_ref()).ok_or_else(|| {
            failure(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::DependencyUnavailable,
            )
        })
    }
}

/// 新建时就公开只给人自己建：Agent 替人建的房间照旧私密、加密，公开要房主自己在房间设置里点。
/// 公开设置本身也得是公开的。
pub(super) fn ensure_public_creator(
    request: &CreatePrivateRoom,
    operation: &'static str,
) -> PrivateRoomResult<()> {
    let Some(settings) = request.listing else {
        return Ok(());
    };
    if settings.listed() && !request.actor.is_agent() && request.created_by_agent.is_none() {
        Ok(())
    } else {
        Err(failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::InvalidRequest,
        )
        .rejecting("listing"))
    }
}

fn conflict(operation: &'static str) -> PrivateRoomFailure {
    failure(
        operation,
        PrivateRoomFailureStage::Persistence,
        PrivateRoomFailureKind::Conflict,
    )
}

fn limit_reached(operation: &'static str) -> PrivateRoomFailure {
    failure(
        operation,
        PrivateRoomFailureStage::Validation,
        PrivateRoomFailureKind::PublicRoomLimitReached,
    )
}
