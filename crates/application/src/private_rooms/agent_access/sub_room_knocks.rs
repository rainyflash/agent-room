//! 子房间第 2f-2a 步（`specs/redesign/sub-rooms.md` 的“加入和退出”）：凭口令、敲门进了父房间的
//! Agent 成员拿房间号进子房间。没有门的直接记成这一间的 Agent 成员；要批准的敲门，等管得了这一间的人
//! 放行；只能请进来的、不在名单里显示的不收敲门，由管得了这一间的人按名字放行（不用先有敲门）。不是
//! 父房间里的 Agent 成员的，说的和没有这间房一样。
//!
//! 管得了这一间的人和人的申请那边一样（[`manages_sub_room`]）：父房间的房主和管理员，进没进来都算，
//! 加上这一间的子房间管理员。替人操作的 Agent 管子房间放到 2h。
//!
//! 父房间里移出 Agent 成员时，它在的子房间里先踢出去（2f-2b-1，`kick_from_sub_rooms`）。
//!
//! 凭口令进了父房间的本机 Agent 没有敲门这条路，它进子房间时由进门那一步问这里
//! （[`SubRoomAgentMemberSeating`]，2f-2b-2）：同一条规矩，没有门的直接记上；要等放行的说门关着，
//! 由管得了这一间的人按名字放行。

use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    join_codes::{PrivateRoomAgentJoinedVia, PrivateRoomAgentMemberStatus},
    private_rooms::{
        PrivateRoomLifecycleStatus, PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
    sub_rooms::{SubRoomAgentKnock, SubRoomDoor, manages_sub_room, sub_room_agent_knock},
    time::UtcMillis,
};

use super::{
    AgentAccessFailure, AgentAccessFailureKind, AgentAccessResult, KnockResult,
    PrivateRoomAgentAccessService, PrivateRoomSnapshot, domain, matrix, matrix_room_id,
    redeemed_room, repository,
};
use crate::{
    ports::{PortFuture, PrivateMatrixMembership, PrivateRoomAgentMemberRecord},
    private_rooms::PrivateRoomActor,
};

/// 本机 Agent 自己进子房间时，它这个 Agent 成员能不能进（2f-2b-2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMemberSeat {
    /// 是这一间已加入的 Agent 成员：本来就是，或者这一间没有门、刚记上。
    Inside,
    /// 这一间有门（要批准、只能请进来），或者它在这一间里被移出过：要管得了这一间的人按名字放它
    /// 进来。
    DoorClosed,
    /// 进不来：不是子房间、它不是父房间里还在的 Agent 成员，或者两间房有一间归档了。
    Refused,
}

/// 凭口令、敲门进了父房间的本机 Agent 进子房间之前，照门把它记成这一间的 Agent 成员。和网络 Agent
/// 敲门是同一条规矩（[`sub_room_agent_knock`]），只是本机 Agent 没有敲门这条路，要等放行的就说门关着。
pub trait SubRoomAgentMemberSeating: Send + Sync {
    fn seat_agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, AgentAccessResult<AgentMemberSeat>>;
}

impl SubRoomAgentMemberSeating for PrivateRoomAgentAccessService {
    fn seat_agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, AgentAccessResult<AgentMemberSeat>> {
        Box::pin(self.seat_agent_member_internal(catalog_id, agent_id))
    }
}

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

    /// 它在这一间里的 Agent 成员记录是什么状态；没有记录时为 `None`。
    async fn member_status(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        operation: &'static str,
    ) -> AgentAccessResult<Option<PrivateRoomAgentMemberStatus>> {
        Ok(self
            .access
            .agent_member(catalog_id, agent_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .map(|member| member.status))
    }

    /// 没有门的子房间直接把它记成 Agent 成员。直接放进来的照样记成敲门进来的（`joined_via` 不加新值），
    /// 和公开房间一样。存储写子房间时锁住父房间的那一条再看（2f-2b-1），父房间里这当口移出了它的不记。
    async fn admit_without_door(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
        operation: &'static str,
    ) -> AgentAccessResult<()> {
        self.access
            .admit_agent(
                catalog_id,
                agent_id,
                PrivateRoomPermissions::AGENT_MEMBER,
                PrivateRoomAgentJoinedVia::Knock,
                now,
            )
            .await
            .map_err(|error| repository(operation, &error))
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
        let status = self.member_status(catalog_id, agent_id, operation).await?;
        if status == Some(PrivateRoomAgentMemberStatus::Joined) {
            return Ok(KnockResult::Member(redeemed_room(sub_room)));
        }
        let removed = status == Some(PrivateRoomAgentMemberStatus::Removed);
        match sub_room_agent_knock(gate.door, removed) {
            SubRoomAgentKnock::Admit => {
                self.admit_without_door(catalog_id, agent_id, now, operation)
                    .await?;
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

    /// 本机 Agent 自己进子房间（2f-2b-2）：它是父房间里还在的 Agent 成员时，照门把它记成这一间的
    /// Agent 成员。两间房有一间不在用、它不在父房间里的都说进不来，和找不到一样，不露出藏起来的房间。
    async fn seat_agent_member_internal(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> AgentAccessResult<AgentMemberSeat> {
        const OPERATION: &str = "sub_room.seat_agent_member";
        let Some(sub_room) = self
            .rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
        else {
            return Ok(AgentMemberSeat::Refused);
        };
        if !sub_room.room().admits_agent_member(true) {
            return Ok(AgentMemberSeat::Refused);
        }
        let gate = match self.sub_room_gate(&sub_room, OPERATION).await {
            Ok(gate) => gate,
            Err(failure) if failure.kind() == AgentAccessFailureKind::NotFound => {
                return Ok(AgentMemberSeat::Refused);
            }
            Err(failure) => return Err(failure),
        };
        if !self.in_parent(&gate, agent_id, OPERATION).await? {
            return Ok(AgentMemberSeat::Refused);
        }
        let status = self.member_status(catalog_id, agent_id, OPERATION).await?;
        if status == Some(PrivateRoomAgentMemberStatus::Joined) {
            return Ok(AgentMemberSeat::Inside);
        }
        let removed = status == Some(PrivateRoomAgentMemberStatus::Removed);
        match sub_room_agent_knock(gate.door, removed) {
            SubRoomAgentKnock::Admit => {
                self.admit_without_door(catalog_id, agent_id, self.clock.now(), OPERATION)
                    .await?;
                // 父房间里这当口移出了它的，存储没记：照进不来说。
                let joined = self.member_status(catalog_id, agent_id, OPERATION).await?
                    == Some(PrivateRoomAgentMemberStatus::Joined);
                Ok(if joined {
                    AgentMemberSeat::Inside
                } else {
                    AgentMemberSeat::Refused
                })
            }
            SubRoomAgentKnock::Wait | SubRoomAgentKnock::InviteOnly => {
                Ok(AgentMemberSeat::DoorClosed)
            }
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

    /// 父房间里要移出这个 Agent 成员：先把它从它在的子房间里踢出去（2f-2b-1），归档了的也踢。只踢，不设
    /// 发言级别：设一次就把归档的房间解开了。没在聊天服务器上那间房里的不踢（只记了账、还没进去）。
    /// 记账时存储连这几间的记录一起改成已移出。
    pub(super) async fn kick_from_sub_rooms(
        &self,
        parent: RoomCatalogId,
        member: &PrivateRoomAgentMemberRecord,
        operation: &'static str,
    ) -> AgentAccessResult<()> {
        let sub_rooms = self
            .sub_room_doors
            .agent_sub_rooms(parent, member.agent_id)
            .await
            .map_err(|error| repository(operation, &error))?;
        for catalog_id in sub_rooms {
            let Some(sub_room) = self
                .rooms
                .find_by_catalog(catalog_id)
                .await
                .map_err(|error| repository(operation, &error))?
            else {
                continue;
            };
            let matrix_room = matrix_room_id(&sub_room, operation)?;
            let membership = self
                .matrix
                .membership(&matrix_room, &member.matrix_user_id)
                .await
                .map_err(|error| matrix(operation, &error))?;
            if matches!(
                membership,
                Some(PrivateMatrixMembership::Joined | PrivateMatrixMembership::Invited)
            ) {
                self.matrix
                    .kick(&matrix_room, &member.matrix_user_id)
                    .await
                    .map_err(|error| matrix(operation, &error))?;
            }
        }
        Ok(())
    }
}
