//! 房间里的人叫什么、Agent 是谁的（`specs/room-people/design.md`）。

use agent_room_domain::ids::{AgentId, PrincipalId, RoomCatalogId};

use crate::{persistence::RepositoryResult, ports::PortFuture};

/// 一个人和他的显示名（注册时填的昵称）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPersonName {
    pub principal_id: PrincipalId,
    pub display_name: String,
}

/// 本机的 Agent 有人类主人；网络 Agent 替它记账的是一个合成主体，没有人类主人。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomAgentKind {
    Local,
    Network,
}

/// 此刻在房间里的一个 Agent 是谁的，按服务器的记录，不看 Agent 自己写的名片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAgentOwnerRecord {
    pub agent_id: AgentId,
    pub kind: RoomAgentKind,
    /// 最早的那位所有者；网络 Agent 为空。
    pub owner: Option<RoomPersonName>,
    /// 私人房间里经敲门放行的，放行的人。
    pub admitted_by: Option<RoomPersonName>,
}

pub trait RoomPeopleDirectory: Send + Sync {
    /// 私人房间全部成员（含受邀、被移出、被封禁的）的名字，按成为成员的时间排。
    fn member_names(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomPersonName>>>;

    /// 这些 Agent 里此刻在这个房间的，各是谁的：本机 Agent 按服务器上已确认的位置算，网络 Agent
    /// 按它在的房间算。不在这个房间的不回答。
    fn agent_owners<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        agent_ids: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<RoomAgentOwnerRecord>>>;
}
