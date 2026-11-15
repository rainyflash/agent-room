//! 子房间第 2h-2 步（`specs/redesign/sub-rooms.md` 的“Agent”）：网络 Agent 看一间房下面的子房间。
//!
//! 看的规矩和网页上“浏览子房间”一样：父房间里在的才看得到名单；勾了“不在名单里显示”的，只有管父房间
//! 的、在里面的看得到。网络 Agent 有两种在父房间里的样子：房间是它建的，它的合成主体就是房主，照房主看；
//! 凭口令或敲门进的，它是那里已加入的 Agent 成员，藏起来的只看得到它建的和它在里面的。
//!
//! 它在不在哪一间里由网络 Agent 那边按它进过的房间说：它的合成主体在它建的房间下面每一间都记着房主，
//! 不能拿来当它进没进去。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    join_codes::PrivateRoomAgentMemberStatus,
    sub_rooms::runs_parent_room,
};

use crate::ports::{PortFuture, PrivateRoomSnapshot, SubRoomEntry};

use super::{
    super::{PrivateRoomFailureStage, PrivateRoomResult, failure::repository},
    PrivateRoomService,
    sub_rooms::{has_joined, not_found, visible},
};

/// 网络 Agent 要看哪间房下面的子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAgentSubRooms {
    /// 替它记账的合成主体。
    pub viewer: PrincipalId,
    pub agent_id: AgentId,
    /// 父房间，或者它下面的任何一间子房间。
    pub catalog_id: RoomCatalogId,
    /// 要“已归档”的那些；不要时只有没归档的。
    pub archived: bool,
}

/// 名单里的一间：它在哪个分类里，写名字；“未分类”为空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSubRoomItem {
    pub entry: SubRoomEntry,
    pub category: Option<String>,
}

/// 一间房下面 Agent 看得到的子房间，按建的先后；分类的名字按排好的先后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSubRooms {
    pub parent: PrivateRoomSnapshot,
    pub categories: Vec<String>,
    pub rooms: Vec<AgentSubRoomItem>,
}

/// 网络 Agent 看子房间名单这一样。网络 Agent 的用例只拿这个。
pub trait AgentSubRoomListing: Send + Sync {
    fn agent_sub_rooms(
        &self,
        request: ListAgentSubRooms,
    ) -> PortFuture<'_, PrivateRoomResult<AgentSubRooms>>;
}

impl AgentSubRoomListing for PrivateRoomService {
    fn agent_sub_rooms(
        &self,
        request: ListAgentSubRooms,
    ) -> PortFuture<'_, PrivateRoomResult<AgentSubRooms>> {
        Box::pin(self.agent_sub_rooms_internal(request))
    }
}

impl PrivateRoomService {
    async fn agent_sub_rooms_internal(
        &self,
        request: ListAgentSubRooms,
    ) -> PrivateRoomResult<AgentSubRooms> {
        const OPERATION: &str = "sub_room.list_by_agent";
        let room = self.load(request.catalog_id, OPERATION).await?;
        let parent = match room.parent_catalog_id() {
            Some(parent) => self.load(parent, OPERATION).await?,
            None => room,
        };
        let parent_id = parent.catalog().id();
        let as_owner = has_joined(parent.room(), request.viewer);
        if !as_owner
            && !self
                .agent_inside(parent_id, request.agent_id, OPERATION)
                .await?
        {
            return Err(not_found(OPERATION));
        }
        let runs_parent = as_owner && runs_parent_room(parent.room(), request.viewer);
        let entries = self
            .sub_rooms
            .list_sub_rooms(parent_id, request.viewer)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        let mut shown = Vec::new();
        for entry in entries {
            if entry.archived != request.archived {
                continue;
            }
            // 藏起来的：管父房间的、建它的、在里面的看得到。在不在里面，Agent 成员按它自己的记录。
            if visible(&entry, runs_parent)
                || entry.created_by == request.viewer
                || (!as_owner
                    && self
                        .agent_inside(entry.catalog_id, request.agent_id, OPERATION)
                        .await?)
            {
                shown.push(entry);
            }
        }
        let categories = self
            .sub_rooms
            .sub_room_categories(parent_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        let rooms = shown
            .into_iter()
            .map(|entry| AgentSubRoomItem {
                category: entry.category_id.and_then(|id| {
                    categories
                        .iter()
                        .find(|category| category.id == id)
                        .map(|category| category.name.clone())
                }),
                entry,
            })
            .collect();
        Ok(AgentSubRooms {
            parent,
            categories: categories
                .into_iter()
                .map(|category| category.name)
                .collect(),
            rooms,
        })
    }

    /// 这个 Agent 是这一间里已加入的 Agent 成员。
    pub(super) async fn agent_inside(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        operation: &'static str,
    ) -> PrivateRoomResult<bool> {
        Ok(self
            .agent_members
            .agent_member_in(catalog_id, agent_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?
            .is_some_and(|member| member.status == PrivateRoomAgentMemberStatus::Joined))
    }
}
