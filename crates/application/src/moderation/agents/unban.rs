//! Agent 解封就是撤回 Agent 做的那条封禁（`specs/public-rooms/design.md`“房主的 Agent 管房间”）。人封的
//! Agent 解不了，要人自己在台账或成员里解。解封不另记一条动作：台账里那条封禁记成撤回，照成员一节那一套
//! 解开，另记是哪个 Agent、按哪份授权、它写的理由，和撤回同一个事务。

use agent_room_domain::{
    ids::PrincipalId,
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditOutcome, ModerationTarget,
    },
    private_rooms::PrivateRoomMembershipStatus,
};

use crate::{
    moderation::service::member_target,
    ports::{AgentModerationAttribution, AgentModerationReversal, RoomModerationGrant},
};

use super::{
    AgentModerate, AgentModerationFailureKind, AgentModerationResult, AgentModerationService,
    failure, ledger, repository,
};

impl AgentModerationService {
    pub(super) async fn unban(
        &self,
        request: &AgentModerate,
        grant: &RoomModerationGrant,
        target: &ModerationTarget,
        operation: &'static str,
    ) -> AgentModerationResult<ModerationAction> {
        if let Some(action) = self.replayed_unban(request, grant, operation).await? {
            return Ok(action);
        }
        let context = self
            .granter_context(grant, ModerationActionKind::Ban, target, operation)
            .await?;
        let bans = self.agent_bans_on(grant, target, operation).await?;
        let moderation = &self.moderation;
        let rooms = moderation
            .effect_rooms(&context, ModerationActionKind::Ban, target, operation)
            .await
            .map_err(ledger)?;
        let mut reversed = None;
        for ban in bans {
            if moderation
                .reverse_effects(&ban, &context, &rooms)
                .await
                .is_err()
            {
                return Err(ledger(moderation.reverse_failed(&ban, operation).await));
            }
            reversed = Some(self.record_unban(request, grant, ban, operation).await?);
        }
        reversed.ok_or_else(|| failure(operation, AgentModerationFailureKind::Internal))
    }

    /// 同一个解封请求再来一次：交回上次撤回的那条。请求 ID 是别的 Agent 用过的，当成冲突。
    async fn replayed_unban(
        &self,
        request: &AgentModerate,
        grant: &RoomModerationGrant,
        operation: &'static str,
    ) -> AgentModerationResult<Option<ModerationAction>> {
        let Some(reversal) = self
            .store
            .agent_reversal(request.action_id)
            .await
            .map_err(|error| repository(operation, &error))?
        else {
            return Ok(None);
        };
        if reversal.attribution.agent_id != grant.agent_id {
            return Err(failure(operation, AgentModerationFailureKind::Conflict));
        }
        self.moderation
            .repository
            .find_action(reversal.attribution.action_id)
            .await
            .map_err(|error| repository(operation, &error))
    }

    /// 封着他的、还没撤的封禁，都得是 Agent 做的。一条都没有时看成员记录：封着就是人在成员里封的。
    async fn agent_bans_on(
        &self,
        grant: &RoomModerationGrant,
        target: &ModerationTarget,
        operation: &'static str,
    ) -> AgentModerationResult<Vec<ModerationAction>> {
        let subject = member_target(target)
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::Internal))?;
        let bans: Vec<_> = self
            .store
            .applied_bans(grant.catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .into_iter()
            .filter(|ban| member_target(ban.action.target()) == Some(subject))
            .collect();
        if bans.iter().any(|ban| !ban.by_agent) {
            return Err(failure(
                operation,
                AgentModerationFailureKind::BannedByPerson,
            ));
        }
        if bans.is_empty() {
            return Err(failure(
                operation,
                if self.banned_in_members(grant, subject, operation).await? {
                    AgentModerationFailureKind::BannedByPerson
                } else {
                    AgentModerationFailureKind::NotBanned
                },
            ));
        }
        Ok(bans.into_iter().map(|ban| ban.action).collect())
    }

    async fn banned_in_members(
        &self,
        grant: &RoomModerationGrant,
        subject: PrincipalId,
        operation: &'static str,
    ) -> AgentModerationResult<bool> {
        let snapshot = self
            .rooms
            .find_by_catalog(grant.catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| failure(operation, AgentModerationFailureKind::NotFound))?;
        Ok(snapshot
            .room()
            .member(subject)
            .is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned))
    }

    /// 解开以后记成撤回，写 `moderation.action.reversed`，同一个事务里记下是这个 Agent 撤的。刚好被别人
    /// 撤掉了（到期、人在台账里点了）就交回当前记录，不再记是它撤的。
    async fn record_unban(
        &self,
        request: &AgentModerate,
        grant: &RoomModerationGrant,
        mut ban: ModerationAction,
        operation: &'static str,
    ) -> AgentModerationResult<ModerationAction> {
        let moderation = &self.moderation;
        let now = moderation.clock.now();
        ban.reverse(now)
            .map_err(|_| failure(operation, AgentModerationFailureKind::Conflict))?;
        let audit = moderation
            .action_audit(
                &ban,
                "moderation.action.reversed",
                ModerationAuditOutcome::Allowed,
                moderation.identifiers.moderation_audit_event_id(),
                now,
                operation,
            )
            .map_err(ledger)?;
        let reversal = AgentModerationReversal {
            request_id: request.action_id,
            attribution: AgentModerationAttribution {
                action_id: ban.id(),
                agent_id: grant.agent_id,
                grant_id: grant.id,
                note: request.note.clone(),
            },
        };
        match self
            .store
            .finalize_agent_reversal(&ban, &audit, &reversal)
            .await
        {
            Ok(reversed) => Ok(reversed),
            Err(error) => moderation
                .ended_meanwhile(ban.id(), &error, operation)
                .await
                .map_err(ledger),
        }
    }
}
