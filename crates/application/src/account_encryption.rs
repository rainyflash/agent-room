//! 人的设备自动签名（ADR 0011，`specs/device-signing/design.md`）：控制面替账户保管密钥存储钥匙，
//! 只交给本人的登录会话；设备要重建签名身份时，由控制面以应用服务的身份代传新的签名公钥。

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use crate::{
    authentication::AuthenticatedPrincipal,
    ports::{
        AccountEncryptionKeyRepository, AccountEncryptionKeySealer, Clock, MatrixCrossSigningKeys,
        MatrixCrossSigningResetGateway, MatrixUserId, StoredEncryptionKey,
    },
};

/// 钥匙是 32 字节：Matrix 密钥存储用的 AES 密钥。
pub const ENCRYPTION_KEY_BYTES: usize = 32;
/// 钥匙 ID 最长这么多字节。
pub const ENCRYPTION_KEY_ID_BYTES: usize = 255;
/// 一小时里最多重建几次签名身份：两台设备来回抢、客户端出了循环时挡一挡。
pub const RESETS_PER_HOUR: usize = 3;
const HOUR_MILLIS: i64 = 3_600_000;

/// 钥匙的明文，丢掉时清零；调试输出里不出现内容。
pub struct EncryptionKeyBytes(Vec<u8>);

impl EncryptionKeyBytes {
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for EncryptionKeyBytes {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&self.0);
    }
}

impl fmt::Debug for EncryptionKeyBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "EncryptionKeyBytes([{} 字节，已隐藏])",
            self.0.len()
        )
    }
}

/// 交给本人的钥匙。
#[derive(Debug)]
pub struct AccountEncryptionKey {
    pub key_id: String,
    pub key: EncryptionKeyBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountEncryptionFailure {
    /// 钥匙不是 32 字节，或钥匙 ID 不对。
    InvalidKey,
    /// 新的签名公钥不对：不是那三把钥匙、缺主密钥，或者有一把不属于本人。
    InvalidCrossSigningKeys,
    /// 重建得太勤：`retry_at` 之后再来。
    RateLimited { retry_at: UtcMillis },
    /// 封存密钥没配，或数据库、Synapse 暂时不可用。
    Unavailable,
}

pub struct AccountEncryptionDependencies {
    pub repository: Arc<dyn AccountEncryptionKeyRepository>,
    pub sealer: Arc<dyn AccountEncryptionKeySealer>,
    pub matrix: Arc<dyn MatrixCrossSigningResetGateway>,
    pub clock: Arc<dyn Clock>,
}

pub struct AccountEncryptionService {
    repository: Arc<dyn AccountEncryptionKeyRepository>,
    sealer: Arc<dyn AccountEncryptionKeySealer>,
    matrix: Arc<dyn MatrixCrossSigningResetGateway>,
    clock: Arc<dyn Clock>,
    /// 每个主体最近一小时的重建时刻。只记在进程里：控制面只有一个实例，重启后从头计也无妨。
    resets: Mutex<HashMap<PrincipalId, Vec<UtcMillis>>>,
}

impl AccountEncryptionService {
    pub fn new(dependencies: AccountEncryptionDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            sealer: dependencies.sealer,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
            resets: Mutex::new(HashMap::new()),
        }
    }

    /// 本人的钥匙；还没有就是 `None`。
    ///
    /// # Errors
    ///
    /// 封存密钥没配、数据库不可用或密文打不开时返回 `Unavailable`。
    pub async fn find(
        &self,
        actor: &AuthenticatedPrincipal,
    ) -> Result<Option<AccountEncryptionKey>, AccountEncryptionFailure> {
        let Some(stored) = self
            .repository
            .find_encryption_key(actor.principal_id)
            .await
            .map_err(|_| AccountEncryptionFailure::Unavailable)?
        else {
            return Ok(None);
        };
        let key = EncryptionKeyBytes::new(
            self.sealer
                .open(actor.principal_id, &stored.sealed)
                .map_err(|_| AccountEncryptionFailure::Unavailable)?,
        );
        if key.expose().len() != ENCRYPTION_KEY_BYTES {
            return Err(AccountEncryptionFailure::Unavailable);
        }
        Ok(Some(AccountEncryptionKey {
            key_id: stored.key_id,
            key,
        }))
    }

    /// 存下或覆盖本人的钥匙。
    ///
    /// # Errors
    ///
    /// 钥匙或 ID 不对时返回 `InvalidKey`；封存或入库失败时返回 `Unavailable`。
    pub async fn store(
        &self,
        actor: &AuthenticatedPrincipal,
        key_id: String,
        key: &EncryptionKeyBytes,
    ) -> Result<(), AccountEncryptionFailure> {
        if !valid_key_id(&key_id) || key.expose().len() != ENCRYPTION_KEY_BYTES {
            return Err(AccountEncryptionFailure::InvalidKey);
        }
        let sealed = self
            .sealer
            .seal(actor.principal_id, key.expose())
            .map_err(|_| AccountEncryptionFailure::Unavailable)?;
        self.repository
            .put_encryption_key(
                actor.principal_id,
                &StoredEncryptionKey { key_id, sealed },
                self.clock.now(),
            )
            .await
            .map_err(|_| AccountEncryptionFailure::Unavailable)
    }

    /// 替本人上传新的签名公钥（设备重建签名身份时）。已有签名身份时换身份要交互认证，由控制面
    /// 以应用服务的身份代传就不用；私钥只在人的设备上。
    ///
    /// # Errors
    ///
    /// 公钥不对时返回 `InvalidCrossSigningKeys`；一小时里已经重建过 [`RESETS_PER_HOUR`] 次时
    /// 返回 `RateLimited`；Synapse 不可用时返回 `Unavailable`。
    pub async fn replace_cross_signing_keys(
        &self,
        actor: &AuthenticatedPrincipal,
        body: serde_json::Value,
    ) -> Result<(), AccountEncryptionFailure> {
        let user = MatrixUserId::new(actor.matrix_user_id.clone())
            .map_err(|_| AccountEncryptionFailure::Unavailable)?;
        let keys = MatrixCrossSigningKeys::new(body, &user)
            .map_err(|_| AccountEncryptionFailure::InvalidCrossSigningKeys)?;
        self.take_reset(actor.principal_id)?;
        self.matrix
            .replace_cross_signing_keys(&user, &keys)
            .await
            .map_err(|_| AccountEncryptionFailure::Unavailable)
    }

    fn take_reset(&self, principal: PrincipalId) -> Result<(), AccountEncryptionFailure> {
        let now = self.clock.now();
        let mut resets = self.resets.lock().unwrap_or_else(PoisonError::into_inner);
        let recent = resets.entry(principal).or_default();
        recent.retain(|at| now.value().saturating_sub(at.value()) < HOUR_MILLIS);
        if recent.len() >= RESETS_PER_HOUR {
            let oldest = recent
                .iter()
                .map(|at| at.value())
                .min()
                .unwrap_or(now.value());
            let retry_at = UtcMillis::new(oldest.saturating_add(HOUR_MILLIS))
                .map_err(|_| AccountEncryptionFailure::Unavailable)?;
            return Err(AccountEncryptionFailure::RateLimited { retry_at });
        }
        recent.push(now);
        Ok(())
    }
}

/// 钥匙 ID 是账户数据里的一段名字：不为空，不超长，没有控制字符。
fn valid_key_id(key_id: &str) -> bool {
    !key_id.is_empty()
        && key_id.len() <= ENCRYPTION_KEY_ID_BYTES
        && !key_id.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "account_encryption/tests.rs"]
mod tests;
