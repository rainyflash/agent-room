//! 私人房间的 Agent 口令与凭口令进来的 Agent 成员。它们放在私人房间聚合之外：口令只影响 Agent
//! 能否入场，不改变任何人的成员资格，也不参与聚合的版本。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    join_codes::PrivateRoomAgentMemberStatus,
    private_rooms::PrivateRoomPermissions,
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixUserId, PortFuture, SecretDigest},
};

/// 房间当前的 Agent 口令。口令本身只在生成时出现一次，这里只有它的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomJoinCodeRecord {
    pub catalog_id: RoomCatalogId,
    pub permissions: PrivateRoomPermissions,
    pub created_by: PrincipalId,
    pub created_at: UtcMillis,
}

/// 一个凭口令进来的 Agent 成员，带上房间设置里展示所需的名字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomAgentMemberRecord {
    pub catalog_id: RoomCatalogId,
    pub agent_id: AgentId,
    pub display_name: String,
    pub matrix_user_id: MatrixUserId,
    /// Agent 的主人的显示名；主人没有显示名时为空。
    pub owner_display_name: Option<String>,
    pub status: PrivateRoomAgentMemberStatus,
    pub permissions: PrivateRoomPermissions,
    pub joined_at: UtcMillis,
    pub status_changed_at: UtcMillis,
}

/// 口令猜错的固定窗口：窗口内失败次数达到上限后，到窗口结束前都不再受理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinCodeAttemptPolicy {
    pub window: DurationMillis,
    pub max_failures: u32,
}

pub trait PrivateRoomAgentAccessStore: Send + Sync {
    fn join_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomJoinCodeRecord>>>;

    /// 换上新口令；房间原来的口令随之失效。
    fn replace_join_code<'a>(
        &'a self,
        record: &'a PrivateRoomJoinCodeRecord,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 停用口令；原本就没有时返回 `false`。
    fn clear_join_code(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RepositoryResult<bool>>;

    fn find_join_code<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomJoinCodeRecord>>>;

    fn agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentMemberRecord>>>;

    /// 房间里全部凭口令进来的 Agent，包括已移出的，按进入时间排列。
    fn agent_members(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomAgentMemberRecord>>>;

    /// 记为已加入；以前被移出的重新加入时更新状态和时间。
    fn admit_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        permissions: PrivateRoomPermissions,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 记为已移出；原本不是已加入时返回 `false`。
    fn remove_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>>;

    /// 这个调用方此刻还能不能试口令：返回 `Some(到期时间)` 表示要等到那时。
    fn join_code_retry_at<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<Option<UtcMillis>>>;

    /// 记一次猜错。
    fn record_join_code_failure<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

/// 只读一个 Agent 成员：内容授权只需要知道某个 Agent 是不是凭口令进来的有效成员。
pub trait PrivateRoomAgentMemberLookup: Send + Sync {
    fn agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentMemberRecord>>>;
}

impl<T: PrivateRoomAgentAccessStore + ?Sized> PrivateRoomAgentMemberLookup for T {
    fn agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentMemberRecord>>> {
        PrivateRoomAgentAccessStore::agent_member(self, catalog_id, agent_id)
    }
}
