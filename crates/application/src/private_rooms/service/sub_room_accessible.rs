//! 子房间第 2f-1b 步（`specs/redesign/sub-rooms.md` 的“接口”）：给 Bridge 的房间列表带子房间
//! （`/lobbies/accessible?subRooms=1`），本机 Agent 照它按 `父房间名/子房间名` 找房间。
//!
//! 只列账号的 Agent 进得去的房间（[`PrivateRoomUseCases::list_for_account`] 给的那些）下面的：
//! 看得到、没归档、没被封禁的子房间，和网页上“浏览子房间”的规矩一样。有门、账号又不在里面的也列，
//! Agent 照着进会被告诉要主人先申请。旧版 Bridge 不带 `subRooms=1`，拿不到它们，也就不会把子房间
//! 当成普通房间摆出来。
//!
//! [`PrivateRoomUseCases::list_for_account`]: super::super::PrivateRoomUseCases::list_for_account

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomMembershipStatus,
    sub_rooms::runs_parent_room,
};

use crate::ports::{PortFuture, PrivateRoomSnapshot, SubRoomEntry};

use super::{
    super::{PrivateRoomFailureStage, PrivateRoomResult, failure::repository},
    PrivateRoomService,
    sub_rooms::visible,
};

/// 账号的 Agent 能按名字找的一间子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibleSubRoom {
    pub parent_catalog_id: RoomCatalogId,
    pub parent_name: String,
    pub entry: SubRoomEntry,
    /// 在哪个分类里，写名字；“未分类”为空。
    pub category: Option<String>,
}

/// 给 Bridge 的房间列表里的子房间。
pub trait AccountSubRoomListing: Send + Sync {
    /// `parents` 是账号的 Agent 进得去的私人房间；回的子房间按父房间的先后，同一间房下面的按建的先后。
    fn accessible_sub_rooms<'a>(
        &'a self,
        principal_id: PrincipalId,
        parents: &'a [PrivateRoomSnapshot],
    ) -> PortFuture<'a, PrivateRoomResult<Vec<AccessibleSubRoom>>>;
}

impl AccountSubRoomListing for PrivateRoomService {
    fn accessible_sub_rooms<'a>(
        &'a self,
        principal_id: PrincipalId,
        parents: &'a [PrivateRoomSnapshot],
    ) -> PortFuture<'a, PrivateRoomResult<Vec<AccessibleSubRoom>>> {
        Box::pin(self.accessible_sub_rooms_internal(principal_id, parents))
    }
}

impl PrivateRoomService {
    async fn accessible_sub_rooms_internal(
        &self,
        principal_id: PrincipalId,
        parents: &[PrivateRoomSnapshot],
    ) -> PrivateRoomResult<Vec<AccessibleSubRoom>> {
        const OPERATION: &str = "sub_room.accessible";
        let mut rooms = Vec::new();
        for parent in parents {
            // 子房间下面没有子房间；只看账号的 Agent 进得去的父房间。
            if parent.parent_catalog_id().is_some() || !parent.room().admits_agent_of(principal_id)
            {
                continue;
            }
            let parent_catalog_id = parent.catalog().id();
            let runs_parent = runs_parent_room(parent.room(), principal_id);
            let entries: Vec<SubRoomEntry> = self
                .sub_rooms
                .list_sub_rooms(parent_catalog_id, principal_id)
                .await
                .map_err(|error| {
                    repository(OPERATION, PrivateRoomFailureStage::Persistence, &error)
                })?
                .into_iter()
                .filter(|entry| {
                    !entry.archived
                        && entry.viewer_status != Some(PrivateRoomMembershipStatus::Banned)
                        && visible(entry, runs_parent)
                })
                .collect();
            // 用得上分类的才去读分类的名字。
            let categories = if entries.iter().any(|entry| entry.category_id.is_some()) {
                self.sub_rooms
                    .sub_room_categories(parent_catalog_id)
                    .await
                    .map_err(|error| {
                        repository(OPERATION, PrivateRoomFailureStage::Persistence, &error)
                    })?
            } else {
                Vec::new()
            };
            rooms.extend(entries.into_iter().map(|entry| AccessibleSubRoom {
                parent_catalog_id,
                parent_name: parent.catalog().name().to_owned(),
                category: entry.category_id.and_then(|id| {
                    categories
                        .iter()
                        .find(|category| category.id == id)
                        .map(|category| category.name.clone())
                }),
                entry,
            }));
        }
        Ok(rooms)
    }
}
