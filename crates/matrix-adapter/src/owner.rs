//! Agent 的主人：授权它的那个人（本机 Bridge 的主人；网络 Agent 没有主人）。
//!
//! 人的设备会自动签名（ADR 0011）：找不到能用的钥匙时，人的设备会重建一次签名身份。以前跟主人
//! 核对过安全码的 Agent 看到的是“核对过的人换了身份”（verification violation）：SDK 不给整个
//! 房间发消息，主人发来的消息也被隔离。重建现在是正常操作，所以对自己的主人，Agent 撤销以前的
//! 核对、记住新身份，照常收发；别人的身份变化仍按 ADR 0009 处理。

use std::sync::{PoisonError, RwLock};

use matrix_sdk::{
    Client,
    encryption::identities::UserIdentity,
    ruma::{OwnedUserId, UserId},
};

#[derive(Debug, Default)]
pub(crate) struct AgentOwner(RwLock<Option<OwnedUserId>>);

impl AgentOwner {
    pub(crate) fn set(&self, owner: OwnedUserId) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(owner);
    }

    pub(crate) fn get(&self) -> Option<OwnedUserId> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn is(&self, user: &UserId) -> bool {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_deref()
            .is_some_and(|owner| owner == user)
    }
}

/// 主人的身份在核对过之后换了：撤销以前的核对、记住新身份。返回这次是不是撤销了。
pub(crate) async fn accept_owner_identity(identity: &UserIdentity) -> bool {
    if !identity.has_verification_violation() {
        return false;
    }
    if let Err(error) = identity.withdraw_verification().await {
        tracing::warn!(%error, "主人换了签名身份，没能撤销以前对他的核对");
        return false;
    }
    if let Err(error) = identity.pin().await {
        tracing::warn!(%error, "撤销了以前对主人的核对，但没能记住他的新身份");
    }
    tracing::info!(
        owner = %identity.user_id(),
        "主人换了签名身份（设备自动签名重建过），撤销了以前对他的核对，照常收发"
    );
    true
}

/// 按本机已知的主人身份检查一遍（不访问服务器），同步之后、判断收到的消息可不可信之前调用。
pub(crate) async fn accept_known_owner_identity(client: &Client, owner: &AgentOwner) -> bool {
    let Some(owner) = owner.get() else {
        return false;
    };
    match client.encryption().get_user_identity(&owner).await {
        Ok(Some(identity)) => accept_owner_identity(&identity).await,
        Ok(None) => false,
        Err(error) => {
            tracing::debug!(%error, "读不到主人的签名身份，下一次同步再看");
            false
        }
    }
}
