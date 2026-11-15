//! 子房间（`specs/redesign/sub-rooms.md`）：父子关系、门、谁在管，和一间房里子房间的三项设置。
//!
//! 子房间自己照旧是一间私人房间，成员、Matrix 房间都走私人房间那一套；这里只管它多出来的几张表。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoom, PrivateRoomMembershipStatus},
    room_listing::RoomJoinMode,
    sub_rooms::{SubRoomDoor, SubRoomPolicy},
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    persistence::RepositoryResult,
    ports::{JoinRequestRecord, PortFuture, PrivateRoomSnapshot, RoomRoleChange},
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
    /// 有门的一律加密；没有门的跟父房间：父房间加密它就加密。
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
    /// 有过门、后来拆了的，第一次拆门的时间：这以后才进来的人不补发以前的房间钥匙。
    pub door_opened_at: Option<UtcMillis>,
    /// 这一间现在加不加密（房间目录上记的）。
    pub end_to_end_encrypted: bool,
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
    /// 看的人在这一间有一个在等的申请时，是什么时候申请的。
    pub viewer_requested_at: Option<UtcMillis>,
    /// 这一间有几个在等的申请。只给管得了这一间的人看。
    pub pending_requests: u32,
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

/// 把一个人记进子房间要存的：改好的子房间，和存之前要再看一眼的那件事。
#[derive(Debug, Clone, Copy)]
pub struct SubRoomAdmission<'a> {
    /// 已经把他记成已加入的子房间。
    pub room: &'a PrivateRoom,
    /// 改之前读到的版本。
    pub expected_version: AggregateVersion,
    pub parent_catalog_id: RoomCatalogId,
    pub principal: PrincipalId,
    pub at: UtcMillis,
    /// 这一次动了谁的身份，和房间在同一个事务里记下。
    pub role_changes: &'a [RoomRoleChange],
}

/// 换门、勾或者去掉“不在名单里显示”：从哪样换成哪样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomDoorChange {
    /// 算新门的时候读到的门。存的时候在事务里再看一眼还是不是它：不是的话，照旧样子算出来的新门
    /// 不能写，别人刚装上的门会被悄悄拆掉，或者反过来。
    pub from: SubRoomDoor,
    pub to: SubRoomDoor,
}

/// 改子房间的名字、用途和门要存的。名字和用途是改完以后的样子，没改的照原来的带上。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomRevision {
    pub catalog_id: RoomCatalogId,
    pub parent_catalog_id: RoomCatalogId,
    pub name: String,
    pub name_key: String,
    pub description: String,
    /// 请求里没提门、也没提隐藏时为空：门那几列不碰。
    pub door: Option<SubRoomDoorChange>,
    pub at: UtcMillis,
}

/// 为什么这间子房间现在改不了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomRevisionBlock {
    /// 这间房里另一间没归档的子房间已经叫这个名字（不分大小写）。
    NameTaken,
    /// 这间子房间归档了、不在了，或者不挂在这间房下面。
    Unavailable,
    /// 要换门，可门已经不是算的时候读到的那样了：另一个请求刚换过。
    DoorChanged,
}

/// 申请进一间子房间，存的时候怎么样了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomJoinRequestOutcome {
    Created,
    /// 他在这一间已经有一个在等的申请：回那一个是什么时候申请的。
    AlreadyPending(UtcMillis),
    /// 这一间此刻不收申请了：拆了门、改成只能请进来、藏起来了，或者归档了。
    Closed,
    /// 他已经不是父房间里已加入的人了。
    NotInParent,
}

/// 子房间跟着父房间变了以后，聊天服务器上还没照着做的一件事（`private_room_sub_room_sync`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomSyncTask {
    pub id: i64,
    /// 哪一间子房间。
    pub catalog_id: RoomCatalogId,
    /// 这个人在这间子房间里的成员记录变了。空：这间子房间自己的事（跟着父房间归档了）。
    pub principal: Option<PrincipalId>,
    /// 领到的是第几版。做完只删这一版：做的时候又排进来的留着。
    pub generation: i64,
    /// 连这一次，一共领过几次。
    pub attempt: u32,
}

/// 领哪些排着的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomSyncScope {
    /// 不分房间：后台每一轮用。
    Everywhere,
    /// 只领这间房下面的子房间的：改父房间的那个请求结束前先做一遍用。
    Under(RoomCatalogId),
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

    /// 把一个人记进子房间。存之前在同一个事务里再看一眼他在父房间里还是不是已加入，不是就什么都
    /// 不写、回 `false`：读到他在父房间里以后、存之前他被移出去的，不能留在子房间里。
    fn admit_to_sub_room<'a>(
        &'a self,
        admission: &'a SubRoomAdmission<'a>,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    /// 改子房间的名字、用途和门：目录上的名字、用途，比重名用的键，门和“不在名单里显示”在同一个
    /// 事务里。从有门改成没有门、以前又没拆过的，记下拆门的时间（`at`）；拆过的不动。换完的门不收
    /// 申请（[`SubRoomDoor::takes_requests`]）的，在等的申请一起作废。撞了名字、子房间归档了、门
    /// 已经不是读到的那样时什么都不写，回原因。
    fn revise_sub_room<'a>(
        &'a self,
        revision: &'a SubRoomRevision,
    ) -> PortFuture<'a, RepositoryResult<Option<SubRoomRevisionBlock>>>;

    /// 申请进一间子房间（`record` 是一个在等的申请）。他在父房间里的那条成员记录和这间子房间的门在
    /// 同一个事务里锁着看：他还是父房间里已加入的人、这一间此刻还收申请（要批准、在名单里显示、没
    /// 归档）才记。同一个人在这一间已经有一个在等的就不再记，回那一个的时间。换门、把他移出父房间
    /// 的事务和这里排着队，它们作废在等的申请时看得到这一条。
    fn request_sub_room_join<'a>(
        &'a self,
        parent_catalog_id: RoomCatalogId,
        record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<SubRoomJoinRequestOutcome>>;

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

    /// 补对账。平时存父房间的那个事务里就把子房间照它改好了；回滚到不认得子房间的旧版那段时间里
    /// 父房间改过的，子房间照到的版本会比父房间旧。这里把这样的子房间找出来，一间一个事务地照父房间
    /// 重算成员记录、房主和状态，聊天服务器上要跟着做的一起排上。一轮最多 `limit` 间，回补了几间。
    fn follow_stale_sub_rooms(
        &self,
        limit: u16,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u32>>;

    /// 领一批排着的事：到了能领的时间的（没领过的、租约过了的、退避到点的），按排进来的先后，最多
    /// `limit` 件。领到的在 `lease_until` 之前别人领不到；做到一半进程断了，过了这个时间别人接着做。
    fn claim_sub_room_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomSyncTask>>>;

    /// 做完了：删掉领到的那一版。做的时候又排进来过的（版本变了，或者已经被别处做完删掉了）再排
    /// 一次：这一处照的可能是旧样子，它之后得还有一遍照新样子做的。
    fn complete_sub_room_sync(&self, task: SubRoomSyncTask)
    -> PortFuture<'_, RepositoryResult<()>>;

    /// 没做成：记下原因，到 `retry_at` 再试。做的时候又排进来过的不记，和做完时一样再排一次。
    fn retry_sub_room_sync(
        &self,
        task: SubRoomSyncTask,
        retry_at: UtcMillis,
        failure_code: &'static str,
    ) -> PortFuture<'_, RepositoryResult<()>>;
}
