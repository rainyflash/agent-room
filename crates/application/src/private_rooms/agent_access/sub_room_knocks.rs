//! 子房间第 2f-2a 步（`specs/redesign/sub-rooms.md` 的“加入和退出”）：凭口令、敲门进了父房间的
//! Agent 成员拿房间号进子房间。没有门的直接记成这一间的 Agent 成员；要批准的敲门，等管得了这一间的人
//! 放行；只能请进来的、不在名单里显示的不收敲门，由管得了这一间的人按名字放行（不用先有敲门）。不是
//! 父房间里的 Agent 成员的，说的和没有这间房一样。
//!
//! 管得了这一间的人和人的申请那边一样（[`manages_sub_room`]）：父房间的房主和管理员，进没进来都算，
//! 加上这一间的子房间管理员。替人操作的 Agent 管子房间放到 2h。

use agent_room_domain::{
    ids::AgentId,
    join_codes::{PrivateRoomAgentJoinedVia, PrivateRoomAgentMemberStatus},
    private_rooms::{
        PrivateRoomLifecycleStatus, PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
    sub_rooms::{SubRoomAgentKnock, SubRoomDoor, manages_sub_room, sub_room_agent_knock},
    time::UtcMillis,
};

use super::{
    AgentAccessFailure, AgentAccessFailureKind, AgentAccessResult, KnockResult,
    PrivateRoomAgentAccessService, PrivateRoomSnapshot, domain, redeemed_room, repository,
};
use crate::private_rooms::PrivateRoomActor;

/// 一间子房间进门要看的：它的父房间、它的门。
struct SubRoomGate {
    parent: PrivateRoomSnapshot,
    door: SubRoomDoor,
}

impl PrivateRoomAgentAccessService {
    /// 父房间和门。这间房其实不是子房间（关系不在了）、父房间找不到的，都当找不到。
    async fn sub_room_gate(
        &self,
        sub_room: &PrivateRoomSnapshot,
        operation: &'static str,
    ) -> AgentAccessResult<SubRoomGate> {
        let not_found = || AgentAccessFailure::new(operation, AgentAccessFailureKind::NotFound);
        let link = self
            .sub_room_doors
            .sub_room_door(sub_room.catalog().id())
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(not_found)?;
        let door = SubRoomDoor::new(link.access, link.hidden)
            .map_err(|error| domain(operation, &error))?;
        let parent = self
            .rooms
            .find_by_catalog(link.parent_catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(not_found)?;
        Ok(SubRoomGate { parent, door })
    }

    /// 这个 Agent 是父房间里还在的 Agent 成员，父房间也还在用。
    async fn in_parent(
        &self,
        gate: &SubRoomGate,
        agent_id: AgentId,
        operation: &'static str,
    ) -> AgentAccessResult<bool> {
        let member = self
            .access
            .agent_member(gate.parent.catalog().id(), agent_id)
            .await
            .map_err(|error| repository(operation, &error))?;
        Ok(gate.parent.room().admits_agent_member(
            member.is_some_and(|member| member.status == PrivateRoomAgentMemberStatus::Joined),
        ))
    }

    /// Agent 拿子房间的房间号敲门。`sub_room` 已经核对过在用。
    pub(super) async fn knock_sub_room(
        &self,
        agent_id: AgentId,
        sub_room: &PrivateRoomSnapshot,
        now: UtcMillis,
        operation: &'static str,
    ) -> AgentAccessResult<KnockResult> {
        let gate = self.sub_room_gate(sub_room, operation).await?;
        if !self.in_parent(&gate, agent_id, operation).await? {
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::NotFound,
            ));
        }
        let catalog_id = sub_room.catalog().id();
        let member = self
            .access
            .agent_member(catalog_id, agent_id)
            .await
            .map_err(|error| repository(operation, &error))?;
        let status = member.map(|member| member.status);
        if status == Some(PrivateRoomAgentMemberStatus::Joined) {
            return Ok(KnockResult::Member(redeemed_room(sub_room)));
        }
        let removed = status == Some(PrivateRoomAgentMemberStatus::Removed);
        match sub_room_agent_knock(gate.door, removed) {
            SubRoomAgentKnock::Admit => {
                // 直接放进来的照样记成敲门进来的（`joined_via` 不加新值），和公开房间一样。
                self.access
                    .admit_agent(
                        catalog_id,
                        agent_id,
                        PrivateRoomPermissions::AGENT_MEMBER,
                        PrivateRoomAgentJoinedVia::Knock,
                        now,
                    )
                    .await
                    .map_err(|error| repository(operation, &error))?;
                Ok(KnockResult::Member(redeemed_room(sub_room)))
            }
            SubRoomAgentKnock::Wait => {
                self.wait_for_manager(catalog_id, agent_id, now, operation)
                    .await
            }
            SubRoomAgentKnock::InviteOnly => Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::InviteOnly,
            )),
        }
    }

    /// 管理者看、回答这间子房间的敲门，移出这一间的 Agent 成员（人，不是替人操作的 Agent）。管得了
    /// 的照 [`manages_sub_room`]；在这一间里、管不了的说没有权限；别的说找不到，藏起来的不露出来。
    /// 这一间或者父房间归档了的说状态变了，和整间房一样。
    pub(super) async fn managed_sub_room(
        &self,
        actor: &PrivateRoomActor,
        sub_room: PrivateRoomSnapshot,
        operation: &'static str,
    ) -> AgentAccessResult<PrivateRoomSnapshot> {
        let gate = self.sub_room_gate(&sub_room, operation).await?;
        let principal = actor.principal_id();
        if !manages_sub_room(gate.parent.room(), sub_room.room(), principal) {
            let inside = sub_room.room().member(principal).is_some_and(|member| {
                matches!(
                    member.status(),
                    PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
                )
            });
            let kind = if inside {
                AgentAccessFailureKind::Forbidden
            } else {
                AgentAccessFailureKind::NotFound
            };
            return Err(AgentAccessFailure::new(operation, kind));
        }
        let active = PrivateRoomLifecycleStatus::Active;
        if sub_room.room().status() != active || gate.parent.room().status() != active {
            return Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::Conflict,
            ));
        }
        Ok(sub_room)
    }

    /// 管理者放行进子房间的 Agent：不要求先有敲门（只能请进来的就是按名字放行），可它得还是父房间里
    /// 的 Agent 成员。
    pub(super) async fn sub_room_admissible(
        &self,
        sub_room: &PrivateRoomSnapshot,
        agent_id: AgentId,
        operation: &'static str,
    ) -> AgentAccessResult<()> {
        let gate = self.sub_room_gate(sub_room, operation).await?;
        if self.in_parent(&gate, agent_id, operation).await? {
            Ok(())
        } else {
            Err(AgentAccessFailure::new(
                operation,
                AgentAccessFailureKind::NotFound,
            ))
        }
    }
}
