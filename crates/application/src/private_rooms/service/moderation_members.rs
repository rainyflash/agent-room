//! 台账里私人房间的踢出、封禁照成员一节那一套落（`specs/public-rooms/design.md`“落到哪”，
//! [`ModerationMemberGateway`]）：改成员记录，在聊天服务器上踢出或封禁，把他名下的 Agent 移出。台账已经
//! 核过动手的人管得了这个房间，这里以房主的身份改成员记录：到期解除时没有动手的人，房主总有“管理”。
//!
//! 他名下凭口令或敲门进来的 Agent 有自己的成员记录（Agent 成员），也照“移出 Agent 成员”那一套记成已移出：
//! 只在聊天服务器上踢的话，它重连时又会进来。被管的是网络 Agent（台账里是替它记账的合成主体）时，他不是
//! 房间里的人，只移出这个 Agent。移出以后旧口令进不来、敲门要管理者放行，所以撤回时不替 Agent 恢复。

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
        ModerationMemberRefusal, PortFuture, PrivateRoomAgentMemberRecord, PrivateRoomSnapshot,
    },
};

use super::{
    super::{
        PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage, PrivateRoomResult,
        failure::{domain, matrix, repository},
    },
    PrivateRoomService, matrix_room_id,
};

/// 被管的人名下此刻在房间里的 Agent 成员，和他是不是网络 Agent 的合成主体。
struct TargetAgents {
    members: Vec<PrivateRoomAgentMemberRecord>,
    network: bool,
}

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
        let agents = self.target_agents(catalog_id, target).await?;
        // 网络 Agent 不是房间里的人：它不在房间里时封禁也没有可落的。
        let gone = agents.members.is_empty()
            && (agents.network
                || (kind == ModerationActionKind::Kick && !in_room(&snapshot, target)));
        Ok(gone.then_some(ModerationMemberRefusal::NotInRoom))
    }

    async fn target_agents(
        &self,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> RepositoryResult<TargetAgents> {
        let owned = self.agents.principal_agents(target).await?;
        let members = if owned.agent_ids.is_empty() {
            Vec::new()
        } else {
            self.agent_members
                .joined_agent_members(catalog_id)
                .await?
                .into_iter()
                .filter(|member| owned.agent_ids.contains(&member.agent_id))
                .collect()
        };
        Ok(TargetAgents {
            members,
            network: owned.network,
        })
    }

    /// 照“移出 Agent 成员”那一套：先在 Matrix 上收紧、踢出，再记成已移出，记账失败时重试还能接着做完。
    async fn evict_agent_members(
        &self,
        snapshot: &PrivateRoomSnapshot,
        members: &[PrivateRoomAgentMemberRecord],
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        if members.is_empty() {
            return Ok(());
        }
        let matrix_room = matrix_room_id(snapshot, operation)?;
        for member in members {
            self.matrix
                .set_speaking(&matrix_room, &member.matrix_user_id, false)
                .await
                .map_err(|error| matrix(operation, error))?;
            self.matrix
                .kick(&matrix_room, &member.matrix_user_id)
                .await
                .map_err(|error| matrix(operation, error))?;
            self.agent_members
                .mark_agent_removed(snapshot.catalog().id(), member.agent_id, self.clock.now())
                .await
                .map_err(|error| {
                    repository(operation, PrivateRoomFailureStage::Persistence, &error)
                })?;
        }
        Ok(())
    }

    async fn moderation_apply(
        &self,
        kind: ModerationActionKind,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "private_room.moderation_apply";
        if matches!(
            kind,
            ModerationActionKind::Hide | ModerationActionKind::Mute
        ) {
            return Err(not_member_action());
        }
        let snapshot = self.load(catalog_id, OPERATION).await?;
        let agents = self
            .target_agents(catalog_id, target)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Directory, &error))?;
        self.evict_agent_members(&snapshot, &agents.members, OPERATION)
            .await?;
        let owner = snapshot.room().owner_principal_id();
        match kind {
            _ if agents.network => Ok(()),
            ModerationActionKind::Kick if in_room(&snapshot, target) => self
                .remove_as(owner, catalog_id, target, OPERATION)
                .await
                .map(drop),
            ModerationActionKind::Kick => Ok(()),
            _ => self
                .ban_as(owner, catalog_id, target, OPERATION)
                .await
                .map(drop),
        }
    }

    async fn moderation_reverse(
        &self,
        kind: ModerationActionKind,
        catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "private_room.moderation_reverse";
        // 网络 Agent 照旧是已移出：要回来就凭新口令或敲门，由管理者决定。
        if self
            .agents
            .principal_agents(target)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Directory, &error))?
            .network
        {
            return Ok(());
        }
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
}

/// 他此刻是不是房间里的人（已加入或受邀）。
fn in_room(snapshot: &PrivateRoomSnapshot, target: PrincipalId) -> bool {
    snapshot.room().member(target).is_some_and(|member| {
        matches!(
            member.status(),
            PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
        )
    })
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
