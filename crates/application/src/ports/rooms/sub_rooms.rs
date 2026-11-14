//! 子房间（`specs/redesign/sub-rooms.md`）：父子关系、门、谁在管，和一间房里子房间的三项设置。
//!
//! 子房间自己照旧是一间私人房间，成员、Matrix 房间都走私人房间那一套；这里只管它多出来的几张表。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomMembershipStatus,
    room_listing::RoomJoinMode,
    sub_rooms::SubRoomPolicy,
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryResult,
    ports::{PortFuture, PrivateRoomSnapshot},
};

/// 建一间子房间要存的：房间本身，加上它挂在谁下面、谁建的、谁在管。
#[derive(Debug, Clone)]
pub struct NewSubRoom {
    pub snapshot: PrivateRoomSnapshot,
    pub parent_catalog_id: RoomCatalogId,
    /// 建的时候父房间的版本：子房间是照这个版本的父房间建出来的。
    pub parent_version: AggregateVersion,
    pub name_key: String,
    pub access: RoomJoinMode,
    pub hidden: bool,
    pub created_by: PrincipalId,
    pub created_by_agent: Option<AgentId>,
    /// 这一间的子房间管理员。建的人自动是；建的人就是父房间的房主时为空。
    pub managers: Vec<PrincipalId>,
    /// 跟父房间：父房间加密它就加密。
    pub end_to_end_encrypted: bool,
    pub created_at: UtcMillis,
}

impl NewSubRoom {
    /// 存之前在事务里再查一遍用的。
    pub fn check(&self) -> SubRoomCreationCheck {
        SubRoomCreationCheck {
            catalog_id: self.snapshot.catalog().id(),
            parent_catalog_id: self.parent_catalog_id,
            name_key: self.name_key.clone(),
            created_by: self.created_by,
            created_by_agent: self.created_by_agent,
            at: self.created_at,
        }
    }
}

/// 这间房下面此刻还建不建得了这个名字的子房间，要查的几样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomCreationCheck {
    /// 要建的这一间。同一个请求重发时它可能已经在了，数名字和个数时不算它自己。
    pub catalog_id: RoomCatalogId,
    pub parent_catalog_id: RoomCatalogId,
    pub name_key: String,
    pub created_by: PrincipalId,
    pub created_by_agent: Option<AgentId>,
    pub at: UtcMillis,
}

/// 为什么这间房下面现在建不了这间子房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomCreationBlock {
    /// 这间房里已经有一间没归档的子房间叫这个名字（不分大小写）。
    NameTaken,
    /// 这间房里没归档的子房间已经到上限。
    RoomLimitReached,
    /// 建的人这一小时建得太多。
    CreatorLimitReached,
    /// 父房间不在了、归档了，或者它自己就是一间子房间。
    ParentUnavailable,
}

/// 建子房间之前要知道的父房间的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomParentFacts {
    pub end_to_end_encrypted: bool,
    pub policy: SubRoomPolicy,
}

/// 一间子房间挂在谁下面、怎么建的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomLink {
    pub catalog_id: RoomCatalogId,
    pub parent_catalog_id: RoomCatalogId,
    pub name_key: String,
    pub access: RoomJoinMode,
    pub hidden: bool,
    pub created_by: PrincipalId,
    pub created_by_agent: Option<AgentId>,
    pub created_at: UtcMillis,
}

/// 子房间名单里的一行，带上看的人自己在这一间里的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomEntry {
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub description: String,
    pub access: RoomJoinMode,
    pub hidden: bool,
    pub archived: bool,
    pub created_by: PrincipalId,
    pub created_at: UtcMillis,
    /// 这一间的子房间管理员，按设的先后。父房间的房主和管理员不在这里。
    pub managers: Vec<PrincipalId>,
    /// 看的人在这一间里的成员状态；从没进过为空。
    pub viewer_status: Option<PrivateRoomMembershipStatus>,
    /// 已加入的人数。
    pub member_count: u32,
    /// 在里面的 Agent 数（本机的和网络的）。
    pub agent_count: u32,
    pub last_message_at: Option<UtcMillis>,
}

/// 存着的三项设置。没定过的房间没有记录，用默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomPolicyRecord {
    pub catalog_id: RoomCatalogId,
    pub policy: SubRoomPolicy,
    pub updated_by: PrincipalId,
    pub updated_at: UtcMillis,
    pub version: AggregateVersion,
}

pub trait SubRoomStore: Send + Sync {
    /// 这间房是子房间时，它的父子关系。
    fn sub_room_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomLink>>>;

    /// 父房间加不加密、三项设置。没有这间私人房间时为空。
    fn sub_room_parent_facts(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomParentFacts>>>;

    /// 建之前先问一遍：重名、到上限、父房间不在了时说出原因，免得白建一间 Matrix 房间。存的时候
    /// 在事务里还会再查。
    fn sub_room_creation_block<'a>(
        &'a self,
        check: &'a SubRoomCreationCheck,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomCreationBlock>>>;

    /// 建一间子房间：房间、父子关系、子房间管理员在同一个事务里。重名、到上限时什么都不写，
    /// 回原因。这间房间号已经有房间时报 `Conflict`（多半是同一个请求的重试抢先存好了）。
    fn create_sub_room<'a>(
        &'a self,
        sub_room: &'a NewSubRoom,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomCreationBlock>>>;

    /// 一间房下面所有的子房间，按建的先后。`viewer` 是看的人。
    fn list_sub_rooms(
        &self,
        parent: RoomCatalogId,
        viewer: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomEntry>>>;

    fn sub_room_policy(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SubRoomPolicyRecord>>>;

    /// 存三项设置。`expected` 是读到的版本，没有记录时为空；别人刚改过就回 `false`，什么都不写。
    fn save_sub_room_policy<'a>(
        &'a self,
        record: &'a SubRoomPolicyRecord,
        expected: Option<AggregateVersion>,
    ) -> PortFuture<'a, RepositoryResult<bool>>;
}
