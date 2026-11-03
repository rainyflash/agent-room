//! 台账里私人房间的踢出、封禁照成员一节那一套落（`specs/public-rooms/design.md`“落到哪”，
//! [`ModerationMemberGateway`]）：改成员记录，在聊天服务器上踢出或封禁，把他名下的 Agent 移出。台账已经
//! 核过动手的人管得了这个房间，这里以房主的身份改成员记录：到期解除时没有动手的人，房主总有“管理”。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    moderation::ModerationActionKind,
    private_rooms::{PrivateRoomLifecycleStatus, PrivateRoomMembershipStatus},
    room_listing::RoomListingSettings,
};

use crate::{
    persistence::RepositoryResult,
    ports::{
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, ModerationMemberGateway,
        ModerationMemberRefusal, PortFuture,
    },
};

use super::{
    super::{
        PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage, PrivateRoomResult,
        failure::{domain, repository},
    },
    PrivateRoomService,
};

impl PrivateRoomService {
    async fn moderation_refusal(
        &self,
        kind: ModerationActionKind,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> RepositoryResult<Option<ModerationMemberRefusal>> {
        let Some(snapshot) = self.store.find_by_catalog(catalog_id).await? else {
            return Ok(Some(ModerationMemberRefusal::RoomUnavailable));
        };
        let room = snapshot.room();
        if room.status() == PrivateRoomLifecycleStatus::Archived {
            return Ok(Some(ModerationMemberRefusal::RoomUnavailable));
        }
        if target == room.owner_principal_id() {
            return Ok(Some(ModerationMemberRefusal::Owner));
        }
        let in_room = room.member(target).is_some_and(|member| {
            matches!(
                member.status(),
                PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
            )
        });
        if kind == ModerationActionKind::Kick && !in_room {
            return Ok(Some(ModerationMemberRefusal::NotInRoom));
        }
        Ok(None)
    }

    async fn moderation_apply(
        &self,
        kind: ModerationActionKind,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "private_room.moderation_apply";
        let owner = self.owner_of(catalog_id, OPERATION).await?;
        match kind {
            ModerationActionKind::Kick => {
                self.remove_as(owner, catalog_id, target, OPERATION).await
            }
            ModerationActionKind::Ban => self.ban_as(owner, catalog_id, target, OPERATION).await,
            ModerationActionKind::Hide | ModerationActionKind::Mute => Err(not_member_action()),
        }
        .map(drop)
    }

    async fn moderation_reverse(
        &self,
        kind: ModerationActionKind,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "private_room.moderation_reverse";
        let snapshot = self.load(catalog_id, OPERATION).await?;
        // 房间关了：没有要恢复的。
        if snapshot.room().status() == PrivateRoomLifecycleStatus::Archived {
            return Ok(());
        }
        let owner = snapshot.room().owner_principal_id();
        match kind {
            ModerationActionKind::Ban => self
                .unban_as(owner, catalog_id, target, OPERATION)
                .await
                .map(drop),
            ModerationActionKind::Kick => {
                self.invite_back(owner, catalog_id, target, OPERATION).await
            }
            ModerationActionKind::Hide | ModerationActionKind::Mute => Err(not_member_action()),
        }
    }

    /// 撤回踢出：照房间新成员的默认权限重新邀请他（没公开过的房间和邀请链接一样：查看、发言、自动发言）。
    /// 他已经回来了、正受邀，或者后来又被封禁了，都不再请。
    async fn invite_back(
        &self,
        owner: PrincipalId,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let snapshot = self.load(catalog_id, operation).await?;
        let gone = snapshot.room().member(target).is_none_or(|member| {
            matches!(
                member.status(),
                PrivateRoomMembershipStatus::Removed | PrivateRoomMembershipStatus::Declined
            )
        });
        if !gone {
            return Ok(());
        }
        let settings = self
            .listings
            .listing(catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))?
            .map_or(RoomListingSettings::UNLISTED, |listing| listing.settings);
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .invite(owner, target, settings.member_permissions())
            .map_err(|error| domain(operation, &error))?;
        self.deliver_invitation(snapshot, room, expected, changed, target, operation)
            .await
            .map(drop)
    }

    async fn owner_of(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrincipalId> {
        Ok(self
            .load(catalog_id, operation)
            .await?
            .room()
            .owner_principal_id())
    }
}

impl ModerationMemberGateway for PrivateRoomService {
    fn check(
        &self,
        kind: ModerationActionKind,
        room_catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationMemberRefusal>>> {
        Box::pin(self.moderation_refusal(kind, room_catalog_id, target))
    }

    fn apply(
        &self,
        kind: ModerationActionKind,
        room_catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PortFuture<'_, MatrixResult<()>> {
        let operation = match kind {
            ModerationActionKind::Ban => MatrixOperation::Ban,
            _ => MatrixOperation::Kick,
        };
        Box::pin(async move {
            self.moderation_apply(kind, room_catalog_id, target)
                .await
                .map_err(|failure| as_matrix_failure(operation, failure))
        })
    }

    fn reverse(
        &self,
        kind: ModerationActionKind,
        room_catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PortFuture<'_, MatrixResult<()>> {
        let operation = match kind {
            ModerationActionKind::Ban => MatrixOperation::Unban,
            _ => MatrixOperation::Invite,
        };
        Box::pin(async move {
            self.moderation_reverse(kind, room_catalog_id, target)
                .await
                .map_err(|failure| as_matrix_failure(operation, failure))
        })
    }
}

const fn not_member_action() -> PrivateRoomFailure {
    PrivateRoomFailure::new(
        "private_room.moderation",
        PrivateRoomFailureStage::Validation,
        PrivateRoomFailureKind::InvalidRequest,
    )
}

/// 台账只关心这一下有没有落成，按最接近的种类报：暂时不通的照旧算不通，到期解除据此退避。
const fn as_matrix_failure(
    operation: MatrixOperation,
    failure: PrivateRoomFailure,
) -> MatrixFailure {
    let kind = match failure.kind() {
        PrivateRoomFailureKind::DependencyUnavailable => MatrixFailureKind::DependencyUnavailable,
        PrivateRoomFailureKind::UnknownCommit => MatrixFailureKind::UnknownCommit,
        PrivateRoomFailureKind::Forbidden => MatrixFailureKind::Forbidden,
        PrivateRoomFailureKind::NotFound => MatrixFailureKind::NotFound,
        PrivateRoomFailureKind::Conflict => MatrixFailureKind::Conflict,
        PrivateRoomFailureKind::InvalidRequest
        | PrivateRoomFailureKind::PublicRoomLimitReached
        | PrivateRoomFailureKind::Internal => MatrixFailureKind::InvalidResponse,
    };
    MatrixFailure::new(operation, kind)
}
