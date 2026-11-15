//! 子房间第 2h-4 步（`specs/redesign/sub-rooms.md` 的“网络 Agent 管子房间”）：网络 Agent 用管房间的
//! 那几个工具管子房间之前，先认出这是它管得了的一间（[`AgentSubRoomManagement`]）。管得了的和归档那一步
//! （2h-3a）一样认：它自己当房主的房间下面每一间，它的合成主体是父房间的房主；凭口令、敲门进了别人的
//! 房间、在下面建的，照“它建的”（`created_by_agent`）认它，还得是这一间和父房间里已加入的 Agent 成员。
//!
//! 放行、移出时 Agent 进门那边（`PrivateRoomAgentAccessService`）照同一条规矩再认一遍。人这边（2h-4b）
//! 在这里做：看在等的申请，把父房间里的人请进来（他在等的申请记成批准）。拒绝留给人做。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomLifecycleStatus,
    room_join_requests::JoinRequestStatus,
    sub_rooms::{authorize_sub_room_admission, manages_sub_room},
};

use crate::ports::{PendingJoinRequest, PortFuture, PrivateRoomSnapshot};

use super::{
    super::{
        PrivateRoomActor, PrivateRoomFailureKind, PrivateRoomFailureStage, PrivateRoomResult,
        SubRoomRefusal,
        failure::{failure, repository},
    },
    PrivateRoomService, ensure_active_room_actor,
    sub_room_members::target_refused,
    sub_room_membership::SubRoomEntrant,
    sub_room_requests::shown_requests,
    sub_rooms::{has_joined, not_found, refused},
};

/// 网络 Agent 要管的一间（2h-4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindAgentManagedSubRoom {
    /// 只认网络 Agent：本机 Agent 建的子房间由主人管。
    pub actor: PrivateRoomActor,
    pub agent_id: AgentId,
    pub catalog_id: RoomCatalogId,
}

/// 网络 Agent 把父房间里的一个人请进它管的子房间（2h-4b）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmitPersonByAgent {
    pub room: FindAgentManagedSubRoom,
    pub target: PrincipalId,
}

/// 网络 Agent 认出它管得了的子房间，看那里人的申请、把父房间里的人请进来。网络 Agent 的用例只拿这个。
pub trait AgentSubRoomManagement: Send + Sync {
    /// 它管得了的这间子房间；这间房不是子房间时回 `None`，照整间房认。在里面却管不了的，和这一间或
    /// 父房间归档了的，说只有管它的人能做（`SubRoomRefusal::ManagersOnly`）；看不到的说找不到，不露出
    /// 藏起来的房间。
    fn agent_managed_sub_room(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Option<PrivateRoomSnapshot>>>;

    /// 这间子房间里在等的人的申请，先来的在前，昵称只留邮箱 @ 前面那段（和人在网页上看的一样）。
    /// 认房间照 [`Self::agent_managed_sub_room`]；不是子房间的回空的：整间房的申请只有公开房间才有。
    fn agent_sub_room_join_requests(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PendingJoinRequest>>>;

    /// 把父房间里的人直接请进来，哪种门都行，和管这一间的人在网页上请进来同一段：被禁言着的不给
    /// 发言级别；他在等的申请记成批准。已经在里面的不算错。他不在父房间里说
    /// `SubRoomRefusal::NotInParent`，在这一间被封禁着说 `SubRoomRefusal::TargetBanned`。不是子房间的
    /// 说找不到。
    fn admit_person_by_agent(
        &self,
        request: AdmitPersonByAgent,
    ) -> PortFuture<'_, PrivateRoomResult<()>>;
}

impl AgentSubRoomManagement for PrivateRoomService {
    fn agent_managed_sub_room(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .agent_managed_family(&request, "sub_room.managed_by_agent")
                .await?
                .map(|family| family.sub_room))
        })
    }

    fn agent_sub_room_join_requests(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PendingJoinRequest>>> {
        Box::pin(self.agent_sub_room_join_requests_internal(request))
    }

    fn admit_person_by_agent(
        &self,
        request: AdmitPersonByAgent,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        Box::pin(self.admit_person_by_agent_internal(request))
    }
}

/// 认出来的子房间和它的父房间。
struct ManagedFamily {
    parent: PrivateRoomSnapshot,
    sub_room: PrivateRoomSnapshot,
}

impl PrivateRoomService {
    async fn agent_managed_family(
        &self,
        request: &FindAgentManagedSubRoom,
        operation: &'static str,
    ) -> PrivateRoomResult<Option<ManagedFamily>> {
        ensure_active_room_actor(&request.actor, self.clock.now(), operation)?;
        if !matches!(request.actor, PrivateRoomActor::NetworkAgent(_)) {
            return Err(failure(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Forbidden,
            ));
        }
        let sub_room = self
            .find(request.catalog_id, operation)
            .await?
            .ok_or_else(|| not_found(operation))?;
        let Some(parent_id) = sub_room.parent_catalog_id() else {
            return Ok(None);
        };
        let parent = self.load(parent_id, operation).await?;
        let viewer = request.actor.principal_id();
        let manages = if has_joined(parent.room(), viewer) {
            // 它自己的房间：合成主体是房主，每一间都管得了。
            manages_sub_room(parent.room(), sub_room.room(), viewer)
        } else {
            let link = self
                .sub_rooms
                .sub_room_link(request.catalog_id)
                .await
                .map_err(|error| {
                    repository(operation, PrivateRoomFailureStage::Persistence, &error)
                })?
                .ok_or_else(|| not_found(operation))?;
            let inside = self
                .agent_inside(request.catalog_id, request.agent_id, operation)
                .await?;
            let created = link.created_by_agent == Some(request.agent_id);
            // 不在父房间里的、藏起来又不是它建的也不在里面的，和没有这间房一样。
            if !self
                .agent_inside(parent_id, request.agent_id, operation)
                .await?
                || (link.hidden && !created && !inside)
            {
                return Err(not_found(operation));
            }
            created && inside
        };
        let active = PrivateRoomLifecycleStatus::Active;
        if !manages || sub_room.room().status() != active || parent.room().status() != active {
            return Err(refused(operation, SubRoomRefusal::ManagersOnly));
        }
        Ok(Some(ManagedFamily { parent, sub_room }))
    }

    async fn agent_sub_room_join_requests_internal(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PrivateRoomResult<Vec<PendingJoinRequest>> {
        const OPERATION: &str = "sub_room.join_request.list_by_agent";
        if self
            .agent_managed_family(&request, OPERATION)
            .await?
            .is_none()
        {
            return Ok(Vec::new());
        }
        let pending = self
            .join_requests
            .pending_join_requests(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        Ok(shown_requests(pending))
    }

    /// 和人请进来（`admit_sub_room_member_internal`）一样先进人、后记申请：反过来的话进人没成，申请
    /// 却已经不在等了。批准的人记它的合成主体。
    async fn admit_person_by_agent_internal(
        &self,
        request: AdmitPersonByAgent,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "sub_room.member.admit_by_agent";
        let now = self.clock.now();
        let ManagedFamily { parent, sub_room } = self
            .agent_managed_family(&request.room, OPERATION)
            .await?
            .ok_or_else(|| not_found(OPERATION))?;
        let permissions =
            authorize_sub_room_admission(parent.room(), sub_room.room(), request.target)
                .map_err(|refusal| target_refused(OPERATION, refusal))?;
        let matrix_user = self
            .principal_matrix_user(request.target, OPERATION)
            .await?;
        self.admit_into_sub_room(
            parent.catalog().id(),
            sub_room,
            &SubRoomEntrant {
                principal: request.target,
                matrix_user: &matrix_user,
                permissions,
            },
            now,
            OPERATION,
        )
        .await
        .map_err(|error| {
            // 读到他在父房间里以后、存之前他被移出去了：说他不在父房间里。房间认过是它管的，这里的
            // “找不到”只会是这个。
            if error.kind() == PrivateRoomFailureKind::NotFound {
                refused(OPERATION, SubRoomRefusal::NotInParent)
            } else {
                error
            }
        })?;
        // 没有在等的、刚被别人处理了的都不算错：他已经在里面了。
        self.join_requests
            .decide_join_request(
                request.room.catalog_id,
                request.target,
                JoinRequestStatus::Approved,
                Some(request.room.actor.principal_id()),
                now,
            )
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        Ok(())
    }
}
