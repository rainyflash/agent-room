//! 台账里同一份授权做的一次全撤（`specs/public-rooms/design.md`“房主的 Agent 管房间”第 7a-3 步）：Agent 照
//! 规则做错了，房主和管理员不用一条一条点。每一条都走人工撤回那一套（要最近登录、确认过影响，禁言先看这个人
//! 此刻该不该禁着，封禁是解封、踢出是请回来），一条没撤成不挡别的。Agent 的解封不算：撤回解封等于再封一次。

use agent_room_domain::{
    ids::{ModerationActionId, RoomCatalogId, RoomModerationGrantId},
    moderation::ModerationAction,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    moderation::{ModerationFailureKind, ModerationUseCases, ReverseModerationAction},
};

use super::{
    AgentModerationFailureKind, AgentModerationResult, AgentModerationService, failure, ledger,
    repository,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverseGrantModeration {
    pub actor: AuthenticatedPrincipal,
    pub room_catalog_id: RoomCatalogId,
    pub grant_id: RoomModerationGrantId,
    pub impact_acknowledged: bool,
}

/// 撤成了的（别人刚好撤掉的照实算在里面），和没撤成的那几条连同台账给的失败种类：再来一次只会再试没撤成的。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrantReversal {
    pub reversed: Vec<ModerationAction>,
    pub failed: Vec<(ModerationActionId, ModerationFailureKind)>,
}

impl AgentModerationService {
    pub(super) async fn reverse_grant_internal(
        &self,
        request: ReverseGrantModeration,
    ) -> AgentModerationResult<GrantReversal> {
        const OPERATION: &str = "agent_moderation.reverse_grant";
        // 一条都没有时也照实报：先看他最近登录过、确认过影响，管得了这个房间。
        if request.actor.expires_at <= self.moderation.clock.now()
            || !request.actor.recently_authenticated
            || !request.impact_acknowledged
        {
            return Err(failure(OPERATION, AgentModerationFailureKind::Forbidden));
        }
        self.moderation
            .require_room_moderator(&request.actor, request.room_catalog_id, OPERATION)
            .await
            .map_err(ledger)?;
        let actions = self
            .store
            .grant_actions(request.room_catalog_id, request.grant_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        let mut outcome = GrantReversal::default();
        for action_id in actions {
            match self
                .moderation
                .reverse_action(ReverseModerationAction {
                    actor: request.actor.clone(),
                    action_id,
                    impact_acknowledged: true,
                })
                .await
            {
                Ok(action) => outcome.reversed.push(action),
                Err(failure) => outcome.failed.push((action_id, failure.kind())),
            }
        }
        Ok(outcome)
    }
}
