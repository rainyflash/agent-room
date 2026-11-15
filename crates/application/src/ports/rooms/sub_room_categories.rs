//! 子房间的分类（`specs/redesign/sub-rooms.md` 的“分类”，第 2e-1 步）：整间房共用的几个组，只有
//! 名字和先后。
//!
//! 建、改、删都先锁父房间在目录里的那一行，和建子房间、改子房间排着队：一间房最多 20 个、名字不重名
//! 数得准，先后改完是从 0 起连着的。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, SubRoomCategoryId},
    time::UtcMillis,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

/// 一个分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomCategory {
    pub id: SubRoomCategoryId,
    pub name: String,
    /// 排在第几个，从 0 起、连着。
    pub position: u8,
}

/// 名字和比重名用的键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomCategoryName {
    pub name: String,
    pub name_key: String,
}

/// 建一个分类要存的。新建的排在最后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSubRoomCategory {
    /// 客户端给的号：同一个请求重发时认得出是同一个。
    pub id: SubRoomCategoryId,
    pub parent_catalog_id: RoomCatalogId,
    pub name: SubRoomCategoryName,
    pub created_by: PrincipalId,
    pub created_at: UtcMillis,
}

/// 改名、挪到第几个。没带的那一项不动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomCategoryRevision {
    pub id: SubRoomCategoryId,
    pub parent_catalog_id: RoomCatalogId,
    pub name: Option<SubRoomCategoryName>,
    /// 挪到第几个；超过最后一个的放到最后。
    pub position: Option<u8>,
    pub at: UtcMillis,
}

/// 删一个分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomCategoryRemoval {
    pub id: SubRoomCategoryId,
    pub parent_catalog_id: RoomCatalogId,
    pub removed_by: PrincipalId,
    pub at: UtcMillis,
}

/// 存完是什么样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubRoomCategoryOutcome {
    /// 存好了，回这间房此刻所有的分类，按先后。
    Saved(Vec<SubRoomCategory>),
    /// 这间房里另一个分类已经叫这个名字（不分大小写）。
    NameTaken,
    /// 这间房的分类已经到上限。
    LimitReached,
    /// 没有这个分类，或者它不在这间房下面。
    Missing,
    /// 这个号已经用来建过别的分类：别的房间的，或者名字不一样。
    IdTaken,
    /// 父房间不在了、归档了，或者它自己是一间子房间。
    ParentUnavailable,
}

pub trait SubRoomCategoryStore: Send + Sync {
    /// 一间房的分类，按先后。
    fn sub_room_categories(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomCategory>>>;

    /// 建一个分类，排在最后，同一个事务里写审计。同一个号、同一间房、同一个名字的已经建过（同一个
    /// 请求重发），什么都不写，回现在的样子。
    fn create_sub_room_category<'a>(
        &'a self,
        category: &'a NewSubRoomCategory,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>>;

    /// 改名、挪位置：挪的时候别的分类跟着让，先后照旧从 0 起连着。
    fn revise_sub_room_category<'a>(
        &'a self,
        revision: &'a SubRoomCategoryRevision,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>>;

    /// 删一个分类，同一个事务里写审计。里面的子房间回到“未分类”（库里的外键置空），后面的分类往前
    /// 挪。已经删掉的再删一次什么都不写，回现在的样子。
    fn delete_sub_room_category<'a>(
        &'a self,
        removal: &'a SubRoomCategoryRemoval,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>>;
}
