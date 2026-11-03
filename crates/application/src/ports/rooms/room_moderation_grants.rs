//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员给自己的一个
//! Agent 的管理授权。授权放在私人房间聚合之外，只记给了哪个 Agent、规则、能做哪几样、到什么时候、
//! 撤了没有。同一个房间、同一个 Agent 同一时间只有一份没撤的，新给的盖掉旧的。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomModerationGrantId},
    room_moderation_grants::{RoomModerationPowers, RoomModerationRules},
    time::UtcMillis,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomModerationGrant {
    pub id: RoomModerationGrantId,
    pub catalog_id: RoomCatalogId,
    pub agent_id: AgentId,
    pub granted_by: PrincipalId,
    pub rules: RoomModerationRules,
    pub powers: RoomModerationPowers,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
    pub revoked_at: Option<UtcMillis>,
}

impl RoomModerationGrant {
    /// 此刻还管用：没撤、没到期。
    #[must_use]
    pub fn is_active_at(&self, now: UtcMillis) -> bool {
        self.revoked_at.is_none() && now < self.expires_at
    }
}

/// 给房主和管理员看的一份在用的授权，带着 Agent 此刻的名字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRoomModerationGrant {
    pub grant: RoomModerationGrant,
    pub agent_display_name: String,
}

pub trait RoomModerationGrantStore: Send + Sync {
    /// 他自己的、还在用的 Agent 叫什么（他是主人或者操作人）；不是他的、停用了的为 `None`。
    fn owned_agent_name(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<String>>>;

    /// 记下一份新的授权；这个房间里这个 Agent 还没撤的那份（到期了的也算）在同一个事务里记成被
    /// `grant.granted_by` 撤了。
    fn replace_room_moderation_grant<'a>(
        &'a self,
        grant: &'a RoomModerationGrant,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 这个房间此刻在用的授权，先给的在前。
    fn active_room_moderation_grants(
        &self,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<ActiveRoomModerationGrant>>>;

    fn room_moderation_grant(
        &self,
        id: RoomModerationGrantId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomModerationGrant>>>;

    /// 撤掉一份。已经撤了的不再改，回 `false`。
    fn revoke_room_moderation_grant(
        &self,
        id: RoomModerationGrantId,
        revoked_by: PrincipalId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>>;
}
