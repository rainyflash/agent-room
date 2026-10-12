//! 私人房间的 Agent 口令、敲门，以及凭口令或放行进来的 Agent 成员。它们放在私人房间聚合之外：
//! 只影响 Agent 能否入场，不改变任何人的成员资格，也不参与聚合的版本。

use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    join_codes::{
        PrivateRoomAgentJoinedVia, PrivateRoomAgentKnockStatus, PrivateRoomAgentMemberStatus,
    },
    private_rooms::PrivateRoomPermissions,
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixUserId, PortFuture, SealedSecret, SecretDigest, SecretSealingFailure},
};

/// 房间当前的 Agent 口令的元数据。口令本身在生成的回答里，管理者再看时从封存的那份解开。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomJoinCodeRecord {
    pub catalog_id: RoomCatalogId,
    pub permissions: PrivateRoomPermissions,
    pub created_by: PrincipalId,
    pub created_at: UtcMillis,
}

/// 房间当前的口令连同摘要和封存的那份（`specs/network-agents/code-first.md`）。封存之前生成的
/// 口令只有摘要，`sealed` 为空；旧版控制面换口令时不写封存的那份，所以解开以后要核对摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedJoinCode {
    pub record: PrivateRoomJoinCodeRecord,
    pub digest: SecretDigest,
    pub sealed: Option<SealedSecret>,
}

/// 用部署配置里的封存密钥封存房间的口令。附加数据绑定房间，密文挪到别的房间打不开。
pub trait JoinCodeSealer: Send + Sync {
    /// # Errors
    ///
    /// 封存密钥没有配置时返回错误。
    fn seal(
        &self,
        catalog_id: RoomCatalogId,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure>;

    /// # Errors
    ///
    /// 封存密钥没有配置、密文被改过或不属于这个房间时返回错误。
    fn open(
        &self,
        catalog_id: RoomCatalogId,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure>;
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

/// 一次敲门：某个 Agent 想进某个私人房间。同一个 Agent 对同一个房间只记一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomAgentKnockRecord {
    pub catalog_id: RoomCatalogId,
    pub agent_id: AgentId,
    /// Agent 当前的显示名。
    pub display_name: String,
    pub status: PrivateRoomAgentKnockStatus,
    /// 最近一次敲门的时刻；再敲一次就从那时重新算。
    pub knocked_at: UtcMillis,
    /// 在等的到这一刻作废。
    pub expires_at: UtcMillis,
    /// 放进来了或没让进的时刻；在等的为空。
    pub decided_at: Option<UtcMillis>,
}

/// 记一次敲门的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateRoomAgentKnockOutcome {
    /// 记下了：新敲的，或者再敲一次（从现在重新算有效期）。
    Waiting(PrivateRoomAgentKnockRecord),
    /// 以前没让进：原样留着，不再打扰管理者。
    Declined(PrivateRoomAgentKnockRecord),
    /// 房间里别的 Agent 在等的已经满了：最早那个在这一刻作废。
    RoomFull { retry_at: UtcMillis },
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

    /// 换上新口令；房间原来的口令随之失效。`sealed` 是封存的口令，封存不了（没配封存密钥）时为空，
    /// 这个口令以后就看不到了。
    fn replace_join_code<'a>(
        &'a self,
        record: &'a PrivateRoomJoinCodeRecord,
        digest: &'a SecretDigest,
        sealed: Option<&'a SealedSecret>,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 房间当前的口令连同摘要和封存的那份，给管理者再看。
    fn sealed_join_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedJoinCode>>>;

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

    /// 记为已加入；以前被移出的重新加入时更新状态、时间和进来的方式。
    fn admit_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        permissions: PrivateRoomPermissions,
        via: PrivateRoomAgentJoinedVia,
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

    /// 记一次敲门，有效期到 `expires_at`。以前没让进的原样留着；房间里别的 Agent 在等、
    /// 没作废的已经有 `max_waiting` 个时不记。
    fn knock(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
        expires_at: UtcMillis,
        max_waiting: u32,
    ) -> PortFuture<'_, RepositoryResult<PrivateRoomAgentKnockOutcome>>;

    /// 房间里在等、没作废的敲门，先敲的在前。停用了的网络 Agent 和不再有效的 Agent 不算。
    fn waiting_knocks(
        &self,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomAgentKnockRecord>>>;

    /// 这个 Agent 在 `since` 以后敲过的门，不论结果，先敲的在前。
    fn agent_knocks(
        &self,
        agent_id: AgentId,
        since: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomAgentKnockRecord>>>;

    /// 把在等、没作废的敲门记为放进来了或没让进；原本不在等时返回 `false`。
    fn decide_knock(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        status: PrivateRoomAgentKnockStatus,
        decided_by: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>>;
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
