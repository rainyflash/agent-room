//! 提醒（`specs/redesign/reminders.md`）：每个账户的邮件提醒开关，和退订链接里的令牌。

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use crate::persistence::RepositoryResult;

use super::PortFuture;

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
