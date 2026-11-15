//! 子房间第 2h-4 步（`specs/redesign/sub-rooms.md` 的“网络 Agent 管子房间”）：网络 Agent 用管房间的
//! 那几个工具管子房间之前，先认出这是它管得了的一间（[`AgentSubRoomManagement`]）。管得了的和归档那一步
//! （2h-3a）一样认：它自己当房主的房间下面每一间，它的合成主体是父房间的房主；凭口令、敲门进了别人的
//! 房间、在下面建的，照“它建的”（`created_by_agent`）认它，还得是这一间和父房间里已加入的 Agent 成员。
//!
//! 放行、移出时 Agent 进门那边（`PrivateRoomAgentAccessService`）照同一条规矩再认一遍。

use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    private_rooms::PrivateRoomLifecycleStatus,
    sub_rooms::manages_sub_room,
};

use crate::ports::{PortFuture, PrivateRoomSnapshot};

use super::{
    super::{
        PrivateRoomActor, PrivateRoomFailureKind, PrivateRoomFailureStage, PrivateRoomResult,
        SubRoomRefusal,
        failure::{failure, repository},
    },
    PrivateRoomService, ensure_active_room_actor,
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

/// 网络 Agent 认出它管得了的子房间。网络 Agent 的用例只拿这个。
pub trait AgentSubRoomManagement: Send + Sync {
    /// 它管得了的这间子房间；这间房不是子房间时回 `None`，照整间房认。在里面却管不了的，和这一间或
    /// 父房间归档了的，说只有管它的人能做（`SubRoomRefusal::ManagersOnly`）；看不到的说找不到，不露出
    /// 藏起来的房间。
    fn agent_managed_sub_room(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Option<PrivateRoomSnapshot>>>;
}

impl AgentSubRoomManagement for PrivateRoomService {
    fn agent_managed_sub_room(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(self.agent_managed_sub_room_internal(request))
    }
}

impl PrivateRoomService {
    async fn agent_managed_sub_room_internal(
        &self,
        request: FindAgentManagedSubRoom,
    ) -> PrivateRoomResult<Option<PrivateRoomSnapshot>> {
        const OPERATION: &str = "sub_room.managed_by_agent";
        ensure_active_room_actor(&request.actor, self.clock.now(), OPERATION)?;
        if !matches!(request.actor, PrivateRoomActor::NetworkAgent(_)) {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Forbidden,
            ));
        }
        let sub_room = self
            .find(request.catalog_id, OPERATION)
            .await?
            .ok_or_else(|| not_found(OPERATION))?;
        let Some(parent_id) = sub_room.parent_catalog_id() else {
            return Ok(None);
        };
        let parent = self.load(parent_id, OPERATION).await?;
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
                    repository(OPERATION, PrivateRoomFailureStage::Persistence, &error)
                })?
                .ok_or_else(|| not_found(OPERATION))?;
            let inside = self
                .agent_inside(request.catalog_id, request.agent_id, OPERATION)
                .await?;
            let created = link.created_by_agent == Some(request.agent_id);
            // 不在父房间里的、藏起来又不是它建的也不在里面的，和没有这间房一样。
            if !self
                .agent_inside(parent_id, request.agent_id, OPERATION)
                .await?
                || (link.hidden && !created && !inside)
            {
                return Err(not_found(OPERATION));
            }
            created && inside
        };
        let active = PrivateRoomLifecycleStatus::Active;
        if !manages || sub_room.room().status() != active || parent.room().status() != active {
            return Err(refused(OPERATION, SubRoomRefusal::ManagersOnly));
        }
        Ok(Some(sub_room))
    }
}
