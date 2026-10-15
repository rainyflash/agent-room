//! 私人房间的邀请链接（`specs/room-people/design.md`）。和 Agent 口令一样放在私人房间聚合之外：
//! 链接本身不改任何人的成员资格，用链接进来时才以创建人的名义做一次普通的邀请。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomPermissions,
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryResult,
    ports::{JoinCodeAttemptPolicy, PortFuture, SealedSecret, SecretDigest},
};

/// 房间当前的邀请链接的元数据。秘密本身在生成的回答里，再看时从封存的那份解开。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomInviteLinkRecord {
    pub catalog_id: RoomCatalogId,
    pub permissions: PrivateRoomPermissions,
    pub created_by: PrincipalId,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
}

/// 房间当前的链接连同摘要和封存的那份。没配封存密钥时 `sealed` 为空，这条链接以后就看不到了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedInviteLink {
    pub record: PrivateRoomInviteLinkRecord,
    pub digest: SecretDigest,
    pub sealed: Option<SealedSecret>,
}

pub trait PrivateRoomInviteLinkStore: Send + Sync {
    /// 换上新链接，房间原来的链接随之失效。
    fn replace_invite_link<'a>(
        &'a self,
        record: &'a PrivateRoomInviteLinkRecord,
        digest: &'a SecretDigest,
        sealed: Option<&'a SealedSecret>,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 房间当前的链接，过期了的也照样回答。
    fn invite_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedInviteLink>>>;

    fn find_invite_link<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomInviteLinkRecord>>>;

    /// 停用链接；原本就没有时返回 `false`。
    fn clear_invite_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>>;

    /// 这个调用方此刻还能不能试链接：返回 `Some(到期时间)` 表示要等到那时。和猜 Agent 口令共用
    /// 一套固定窗口，调用方的键各不相同（`principal:<id>`）。
    fn invite_link_retry_at<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<Option<UtcMillis>>>;

    /// 记一次猜错。
    fn record_invite_link_failure<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}
