//! 身份变动记录（`specs/redesign/room-roles.md` 的“记录”）：谁、什么时候、把谁、从什么身份改成了
//! 什么身份。写在保存房间的同一个事务里，只加不改；房主和管理员读得到。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_role_changes::{RoomStanding, RoomStandingChange},
    time::UtcMillis,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

/// 要记下的一条。时间用保存房间的那一刻。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomRoleChange {
    /// 动手的人。不是谁动的手（房主删除账户，房间交给了管理员）时为空。
    pub actor: Option<PrincipalId>,
    pub change: RoomStandingChange,
}

/// 存下来的一条。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomRoleChangeRecord {
    /// 库里的编号，新的大。
    pub id: i64,
    pub actor: Option<PrincipalId>,
    pub target: PrincipalId,
    /// 之前名单里没有这个人时为空。
    pub previous: Option<RoomStanding>,
    pub next: RoomStanding,
    pub changed_at: UtcMillis,
}

pub trait RoomRoleChangeLog: Send + Sync {
    /// 这间房最近的身份变动，新的在前，最多 `limit` 条。
    fn recent_role_changes(
        &self,
        catalog_id: RoomCatalogId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomRoleChangeRecord>>>;
}
