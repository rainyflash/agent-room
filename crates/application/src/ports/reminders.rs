//! 提醒（`specs/redesign/reminders.md`）：每个账户的邮件提醒开关，和退订链接里的令牌。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use serde_json::Value;

use crate::persistence::RepositoryResult;

use super::{MatrixResult, MatrixRoomId, MatrixUserId, PortFuture};

/// 一个账户此刻的提醒设置。没存过的照默认：邮件提醒开着。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderAccount {
    /// 还在用的账户。被停用的也算：退订照样能退，设置照样记着。
    Active { email_enabled: bool },
    /// 没有这个账户，或者正在删、已经删了。
    Gone,
}

pub trait ReminderSettingsRepository: Send + Sync {
    fn find_reminder_account(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ReminderAccount>>;

    /// 打开或关上邮件提醒；和现在一样就不动，变了才记下改的时间。账户正在删、已经删了的什么都
    /// 不写，回答 `false`。
    fn set_email_reminders(
        &self,
        principal_id: PrincipalId,
        enabled: bool,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>>;
}

/// 退订链接里的令牌：签过名的账户 ID 加上用途，不过期，可以反复用。
pub trait ReminderUnsubscribeTokens: Send + Sync {
    fn issue(&self, principal_id: PrincipalId) -> String;

    /// 签名对得上才认；格式不对、签名不对都是 `None`。
    fn verify(&self, token: &str) -> Option<PrincipalId>;
}

/// 每小时那一轮要看的一个账户（`specs/redesign/reminders.md` 的“怎么算”）：还在用、不是网络 Agent、
/// 邮件提醒开着（没有那一行算开着）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderCandidate {
    pub principal_id: PrincipalId,
    pub matrix_user_id: MatrixUserId,
    /// 上一封提醒什么时候发的；从没发过是 `None`。
    pub last_email_at: Option<UtcMillis>,
    /// 上一封说到哪一刻为止。
    pub covered_until: Option<UtcMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderRoomKind {
    /// 私人房间，公开了的也是。
    Room,
    SubRoom,
    Direct,
}

/// 他此刻在里面、还在用的一间房。不含公共大厅和归档了的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderRoom {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomId,
    pub kind: ReminderRoomKind,
    pub end_to_end_encrypted: bool,
    /// 他什么时候进来的：网页还没写过读到的位置时从这一刻算起。
    pub joined_at: UtcMillis,
}

pub trait ReminderRecipientRepository: Send + Sync {
    /// 按账户 ID 从小到大，`after` 之后的最多 `limit` 个候选。
    fn reminder_candidates(
        &self,
        after: Option<PrincipalId>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderCandidate>>>;

    fn reminder_rooms(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderRoom>>>;
}

/// 房间里别人说的一条话，只留算提醒要用的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderMessage {
    pub event_id: String,
    pub sender: MatrixUserId,
    pub sent_at: UtcMillis,
    /// 不加密的房间里，`conversation.mentions` 有没有他。加密的房间读不到，总是 `false`。
    pub mentions_reader: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderMessagePage {
    /// 从新到旧，`sent_at` 不早于问的那一刻。
    pub messages: Vec<ReminderMessage>,
    /// 翻到上限还没翻到问的那一刻：实际比 `messages` 多。
    pub truncated: bool,
}

/// 以应用服务替这个人读聊天服务器（应用服务的命名空间覆盖所有本地用户，ADR 0011）。
pub trait ReminderMatrixReader: Send + Sync {
    /// 他的个人工作区（`io.github.rainyflash.agentroom.personal-workspace.v1`），原样交回；没写过是
    /// `None`。
    fn personal_workspace<'a>(
        &'a self,
        user: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<Value>>>;

    /// 这间房里 `since`（含）以后的消息：加密的房间是 `m.room.encrypted`，不加密的是消息预览事件。
    /// 包括他自己发的，挑出来是用例的事。
    fn messages_since<'a>(
        &'a self,
        user: &'a MatrixUserId,
        room: &'a MatrixRoomId,
        end_to_end_encrypted: bool,
        since: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<ReminderMessagePage>>;
}
