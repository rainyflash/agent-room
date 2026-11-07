//! 改好密码以后退出别的地方的登录（`specs/change-password/design.md`）：撤销这个人别的账户登录、
//! 结束他在身份服务上的登录、注销他别的聊天设备。正在用的这一份账户登录和这台的聊天设备留着；
//! 电脑授权和 Agent 连接不动。

use std::sync::Arc;

use crate::{
    authentication::AuthenticatedPrincipal,
    ports::{
        Clock, MatrixDeviceId, MatrixHumanDeviceSignOut, MatrixUserId, SecretFactory, SecretValue,
        SignInSessionTermination, WebSessionStore,
    },
};

/// 退出了几处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedOutElsewhere {
    /// 撤了几份账户登录（别的浏览器、别的桌面端）。
    pub account_sessions: u64,
    /// 注销了几台聊天设备。
    pub chat_devices: usize,
    /// 身份服务上的登录结束了没有。没结束成不算失败（账户登录和聊天设备已经退了），由接口记一条
    /// 告警：别处还能凭身份服务上剩下的登录不输密码换到新的账户登录。
    pub sign_in_sessions_ended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignOutElsewhereFailure {
    /// 库暂时不可用，什么都没退。
    AccountSessionsUnavailable,
    /// Synapse 暂时不可用：账户登录已经撤了，再来一次是安全的。
    ChatUnavailable,
}

pub struct SignOutElsewhereService {
    sessions: Arc<dyn WebSessionStore>,
    chat_devices: Arc<dyn MatrixHumanDeviceSignOut>,
    sign_in_sessions: Arc<dyn SignInSessionTermination>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
}

pub struct SignOutElsewhereDependencies {
    pub sessions: Arc<dyn WebSessionStore>,
    pub chat_devices: Arc<dyn MatrixHumanDeviceSignOut>,
    pub sign_in_sessions: Arc<dyn SignInSessionTermination>,
    /// 和认证用的是同一个，算出来的摘要才对得上库里这份登录。
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
}

impl SignOutElsewhereService {
    pub fn new(dependencies: SignOutElsewhereDependencies) -> Self {
        Self {
            sessions: dependencies.sessions,
            chat_devices: dependencies.chat_devices,
            sign_in_sessions: dependencies.sign_in_sessions,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
        }
    }

    /// `actor` 是凭 `session_secret` 认出来的人。先撤账户登录（最要紧，也最不会出错），
    /// 再结束身份服务上的登录（免得别处悄悄又登录回来），最后注销聊天设备。
    ///
    /// # Errors
    ///
    /// 库不可用时返回 `AccountSessionsUnavailable`；Synapse 不可用时返回 `ChatUnavailable`。
    pub async fn sign_out(
        &self,
        actor: &AuthenticatedPrincipal,
        session_secret: &SecretValue,
        keep_chat_device: Option<&MatrixDeviceId>,
    ) -> Result<SignedOutElsewhere, SignOutElsewhereFailure> {
        let keep = self.secrets.digest(session_secret.expose());
        let revoked = self
            .sessions
            .revoke_others(actor.principal_id, Some(&keep), self.clock.now())
            .await
            .map_err(|_| SignOutElsewhereFailure::AccountSessionsUnavailable)?;
        let sign_in_sessions_ended = match &revoked {
            Some(revoked) => self
                .sign_in_sessions
                .end_sessions(&revoked.account)
                .await
                .is_ok(),
            None => false,
        };
        let user = MatrixUserId::new(actor.matrix_user_id.clone())
            .map_err(|_| SignOutElsewhereFailure::ChatUnavailable)?;
        let chat_devices = self
            .chat_devices
            .sign_out_other_devices(&user, keep_chat_device)
            .await
            .map_err(|_| SignOutElsewhereFailure::ChatUnavailable)?;
        Ok(SignedOutElsewhere {
            account_sessions: revoked.map_or(0, |revoked| revoked.revoked),
            chat_devices,
            sign_in_sessions_ended,
        })
    }
}

#[cfg(test)]
#[path = "sign_out_elsewhere/tests.rs"]
mod tests;
