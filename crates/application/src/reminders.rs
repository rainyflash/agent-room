//! 提醒（`specs/redesign/reminders.md` 的“邮件提醒”）：每个账户一个邮件提醒开关，默认开着；邮件里的
//! 退订链接不用登录，凭签过名的令牌关掉它。
//!
//! 4R-3 存开关、出退订页；4R-4 算该提醒什么（[`ReminderPlanner`]，每小时一轮的演练是
//! [`ReminderDryRun`]），还不发信。“这台服务器开没开邮件
//! 提醒”由部署配置定，生产上要等发信那一步（4R-5）维护者点了头才打开；没开时网页不显示这个开关。

mod plan;
mod planner;
mod round;
mod workspace;

use std::sync::Arc;

pub use plan::{
    LOOKBACK_MS, MINIMUM_GAP_MS, ReminderDecision, ReminderHold, ReminderPlan, ReminderRoomTally,
    SETTLE_MS,
};
pub use planner::{ReminderPlanFailure, ReminderPlanner, ReminderPlannerDependencies};
pub use round::{ROUND_BATCH, ReminderDryRun, ReminderRoundReport};
pub use workspace::{Pause, PersonalWorkspace, ReadPosition};

use crate::{
    authentication::AuthenticatedPrincipal,
    ports::{Clock, ReminderAccount, ReminderSettingsRepository, ReminderUnsubscribeTokens},
};

/// 一个账户的提醒设置，设置页读的就是它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReminderSettings {
    pub email: EmailReminderSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmailReminderSettings {
    /// 他自己的开关。
    pub enabled: bool,
    /// 这台服务器开没开邮件提醒。没开时开关存着也不发。
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderFailure {
    /// 数据库暂时不可用，或者账户正在删除。
    Unavailable,
}

/// 退订链接打开时，这个账户现在收不收提醒邮件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsubscribeStanding {
    Subscribed,
    /// 已经退了；账户删掉了的也算。
    Unsubscribed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsubscribeFailure {
    /// 令牌不对，或者这台服务器没配签令牌的钥匙。
    InvalidToken,
    /// 数据库暂时不可用。
    Unavailable,
}

pub struct ReminderDependencies {
    pub repository: Arc<dyn ReminderSettingsRepository>,
    /// 没配签令牌的钥匙时为空：退订链接一律不认。
    pub tokens: Option<Arc<dyn ReminderUnsubscribeTokens>>,
    pub clock: Arc<dyn Clock>,
    pub email_available: bool,
}

pub struct ReminderService {
    repository: Arc<dyn ReminderSettingsRepository>,
    tokens: Option<Arc<dyn ReminderUnsubscribeTokens>>,
    clock: Arc<dyn Clock>,
    email_available: bool,
}

impl ReminderService {
    pub fn new(dependencies: ReminderDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            tokens: dependencies.tokens,
            clock: dependencies.clock,
            email_available: dependencies.email_available,
        }
    }

    /// 本人的提醒设置；没存过的照默认（开着）。
    ///
    /// # Errors
    ///
    /// 数据库不可用时返回 `Unavailable`。
    pub async fn settings(
        &self,
        actor: &AuthenticatedPrincipal,
    ) -> Result<ReminderSettings, ReminderFailure> {
        let account = self
            .repository
            .find_reminder_account(actor.principal_id)
            .await
            .map_err(|_| ReminderFailure::Unavailable)?;
        match account {
            ReminderAccount::Active { email_enabled } => Ok(self.view(email_enabled)),
            ReminderAccount::Gone => Err(ReminderFailure::Unavailable),
        }
    }

    /// 本人打开或关上邮件提醒。服务器没开邮件提醒时也照样存：以后开了就按它来。
    ///
    /// # Errors
    ///
    /// 数据库不可用、账户正在删除时返回 `Unavailable`。
    pub async fn set_email(
        &self,
        actor: &AuthenticatedPrincipal,
        enabled: bool,
    ) -> Result<ReminderSettings, ReminderFailure> {
        let written = self
            .repository
            .set_email_reminders(actor.principal_id, enabled, self.clock.now())
            .await
            .map_err(|_| ReminderFailure::Unavailable)?;
        if written {
            Ok(self.view(enabled))
        } else {
            Err(ReminderFailure::Unavailable)
        }
    }

    /// 打开退订链接时看一眼：令牌对不对、现在还收不收。只读，不改什么：邮箱的链接扫描会先打开一遍。
    ///
    /// # Errors
    ///
    /// 令牌不对时返回 `InvalidToken`；数据库不可用时返回 `Unavailable`。
    pub async fn unsubscribe_standing(
        &self,
        token: &str,
    ) -> Result<UnsubscribeStanding, UnsubscribeFailure> {
        let principal_id = self.verify(token)?;
        let account = self
            .repository
            .find_reminder_account(principal_id)
            .await
            .map_err(|_| UnsubscribeFailure::Unavailable)?;
        Ok(match account {
            ReminderAccount::Active {
                email_enabled: true,
            } => UnsubscribeStanding::Subscribed,
            ReminderAccount::Active {
                email_enabled: false,
            }
            | ReminderAccount::Gone => UnsubscribeStanding::Unsubscribed,
        })
    }

    /// 凭令牌关掉邮件提醒。已经关了、账户删掉了的也当退成了：再点一次还是“已经退了”。
    ///
    /// # Errors
    ///
    /// 令牌不对时返回 `InvalidToken`；数据库不可用时返回 `Unavailable`。
    pub async fn unsubscribe(&self, token: &str) -> Result<(), UnsubscribeFailure> {
        let principal_id = self.verify(token)?;
        self.repository
            .set_email_reminders(principal_id, false, self.clock.now())
            .await
            .map_err(|_| UnsubscribeFailure::Unavailable)?;
        Ok(())
    }

    fn verify(
        &self,
        token: &str,
    ) -> Result<agent_room_domain::ids::PrincipalId, UnsubscribeFailure> {
        self.tokens
            .as_ref()
            .and_then(|tokens| tokens.verify(token))
            .ok_or(UnsubscribeFailure::InvalidToken)
    }

    const fn view(&self, enabled: bool) -> ReminderSettings {
        ReminderSettings {
            email: EmailReminderSettings {
                enabled,
                available: self.email_available,
            },
        }
    }
}

#[cfg(test)]
#[path = "reminders/tests.rs"]
mod tests;
