//! 子房间第 2g-1 步（`specs/redesign/sub-rooms.md` 的“Agent”）：本机 Agent 替它的主人建子房间。
//!
//! 规矩和人在网页上建一样（[`PrivateRoomService::create_sub_room_internal`]）：主人得是父房间里能建的
//! 人，房间的设置得让 Agent 建，Agent 得是主人名下还在用的。只是分类按名字给：Agent 只知道名字，
//! 没有这个分类就放进“未分类”，回答里说清楚，不替它新建分类（分类是管房间的人排的）。

use agent_room_domain::sub_room_categories::sub_room_category_name_key;

use super::{
    super::{
        AgentSubRoomCreated, CreateAgentSubRoom, CreateSubRoom, PrivateRoomFailureStage,
        PrivateRoomResult, failure::repository,
    },
    PrivateRoomService,
};
use crate::ports::PortFuture;

/// Agent 建子房间这一样（第 2g-1、2h 步）。网络 Agent 的用例只拿这个，不拿整个
/// [`super::SubRoomUseCases`]。
pub trait AgentSubRoomCreation: Send + Sync {
    /// 和 [`super::SubRoomUseCases::create_agent_sub_room`] 一样。
    fn create_agent_sub_room(
        &self,
        request: CreateAgentSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<AgentSubRoomCreated>>;
}

impl AgentSubRoomCreation for PrivateRoomService {
    fn create_agent_sub_room(
        &self,
        request: CreateAgentSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<AgentSubRoomCreated>> {
        Box::pin(self.create_agent_sub_room_internal(request))
    }
}

impl PrivateRoomService {
    pub(super) async fn create_agent_sub_room_internal(
        &self,
        request: CreateAgentSubRoom,
    ) -> PrivateRoomResult<AgentSubRoomCreated> {
        const OPERATION: &str = "sub_room.create_by_agent";
        // 读分类在建之前：建的时候要分类号。父房间不存在、他不是里面的人时，建那一步照常拒绝，
        // 读到的分类不往外给。
        let categories = self
            .sub_rooms
            .sub_room_categories(request.parent_catalog_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        let wanted = request.category.as_deref().map(sub_room_category_name_key);
        let chosen = wanted.as_ref().and_then(|wanted| {
            categories
                .into_iter()
                .find(|category| sub_room_category_name_key(&category.name) == *wanted)
        });
        let room = self
            .create_sub_room_internal(CreateSubRoom {
                actor: request.actor,
                parent_catalog_id: request.parent_catalog_id,
                catalog_id: request.catalog_id,
                name: request.name,
                description: String::new(),
                access: request.access,
                hidden: false,
                category_id: chosen.as_ref().map(|category| category.id),
                created_by_agent: Some(request.created_by_agent),
            })
            .await?;
        let parent_name = self
            .load(request.parent_catalog_id, OPERATION)
            .await?
            .catalog()
            .name()
            .to_owned();
        Ok(AgentSubRoomCreated {
            room,
            parent_name,
            category_missing: wanted.is_some() && chosen.is_none(),
            category: chosen.map(|category| category.name),
        })
    }
}
