//! 网络 Agent 看一间房下面的子房间（`specs/redesign/sub-rooms.md` 第 2h-2 步）。
//!
//! 父房间是它建的，或者它凭口令、敲门进了的；看得到哪几间由私人房间用例照网页“浏览子房间”的规矩
//! 定（[`AgentSubRoomListing`]）。它在不在哪一间里，按它进过的房间说。没有门的、它在里面的，再给
//! 里面有几个人、几个 Agent、最近有人说话的时间；有门、它又不在里面的只给名字、用途、分类和门。

use agent_room_domain::{
    ids::{NetworkAgentId, PrincipalId, RoomCatalogId},
    room_listing::RoomJoinMode,
    time::UtcMillis,
};

use super::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentResult, NetworkAgentRoom,
    NetworkAgentService, internal, repository,
    rooms::{owned_room, room_failure, unavailable_parent},
};
use crate::private_rooms::{
    AgentSubRoomItem, ListAgentSubRooms, PrivateRoomFailure, PrivateRoomFailureKind,
};

/// 一间房下面它看得到的子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSubRoomList {
    pub parent: NetworkAgentRoom,
    /// 父房间里的分类，按排好的先后。建子房间时 `category` 写其中一个。
    pub categories: Vec<String>,
    /// 按建的先后。
    pub rooms: Vec<NetworkAgentSubRoomSummary>,
}

/// 名单里的一间子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSubRoomSummary {
    pub room: NetworkAgentRoom,
    pub description: String,
    /// 分类的名字；“未分类”为 `None`。
    pub category: Option<String>,
    pub access: RoomJoinMode,
    /// 它在里面（进过、还没离开）。
    pub joined: bool,
    /// 它建的。
    pub created_by_you: bool,
    /// 没有门的、它在里面的才有。
    pub activity: Option<NetworkAgentSubRoomActivity>,
}

/// 里面有几个人、几个 Agent，最近有人说话的时间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkAgentSubRoomActivity {
    pub people: u32,
    pub agents: u32,
    pub last_message_at: Option<UtcMillis>,
}

impl NetworkAgentService {
    pub(super) async fn list_sub_rooms_internal(
        &self,
        token: &str,
        parent: &str,
        archived: bool,
    ) -> NetworkAgentResult<NetworkAgentSubRoomList> {
        let record = self.authenticated(token).await?;
        let agent_id = record.agent_id.ok_or_else(internal)?;
        self.store
            .record_activity(record.id, self.clock.now())
            .await
            .map_err(repository)?;
        let catalog_id = self
            .room_of(record.id, parent)
            .await
            .map_err(unavailable_parent)?;
        let listed = self
            .sub_room_listing
            .agent_sub_rooms(ListAgentSubRooms {
                viewer: record.principal_id,
                agent_id,
                catalog_id,
                archived,
            })
            .await
            .map_err(listing_failure)?;
        let entered = self.entered_rooms(record.id).await?;
        let rooms = listed
            .rooms
            .into_iter()
            .map(|item| summary(item, &entered, record.principal_id))
            .collect();
        Ok(NetworkAgentSubRoomList {
            parent: owned_room(&listed.parent),
            categories: listed.categories,
            rooms,
        })
    }

    /// 它进过、还没离开的房间。
    async fn entered_rooms(&self, id: NetworkAgentId) -> NetworkAgentResult<Vec<RoomCatalogId>> {
        Ok(self
            .store
            .rooms(id)
            .await
            .map_err(repository)?
            .into_iter()
            .map(|room| room.catalog_id)
            .collect())
    }
}

fn summary(
    item: AgentSubRoomItem,
    entered: &[RoomCatalogId],
    principal_id: PrincipalId,
) -> NetworkAgentSubRoomSummary {
    let entry = item.entry;
    let joined = entered.contains(&entry.catalog_id);
    let activity =
        (joined || entry.access == RoomJoinMode::Open).then_some(NetworkAgentSubRoomActivity {
            people: entry.member_count,
            agents: entry.agent_count,
            last_message_at: entry.last_message_at,
        });
    NetworkAgentSubRoomSummary {
        room: NetworkAgentRoom {
            catalog_id: entry.catalog_id,
            matrix_room_id: entry.matrix_room,
            name: entry.name,
            archived: entry.archived,
        },
        description: entry.description,
        category: item.category,
        access: entry.access,
        joined,
        created_by_you: entry.created_by == principal_id,
        activity,
    }
}

/// 看不了名单：父房间不在、它不在里面了，照建子房间的说法说父房间用不了。
fn listing_failure(failure: PrivateRoomFailure) -> NetworkAgentFailure {
    match failure.kind() {
        PrivateRoomFailureKind::NotFound
        | PrivateRoomFailureKind::Forbidden
        | PrivateRoomFailureKind::OwnerOnly => {
            NetworkAgentFailure::new(NetworkAgentFailureKind::SubRoomParentUnavailable)
        }
        _ => room_failure(failure),
    }
}
