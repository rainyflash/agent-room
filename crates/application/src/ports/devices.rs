use std::fmt;

use agent_room_domain::{
    devices::{Device, DevicePublicSigningKey, DeviceTokenFamily},
    ids::{
        AgentInstanceId, DeviceAccessTokenId, DeviceId, DeviceRefreshAttemptId,
        DeviceRefreshTokenId, OutboxEventId, PrincipalId,
    },
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::{
    PortFuture, PrincipalAccount, PrincipalRegistration, SecretDigest, SecretGenerationFailure,
    SecretValue,
};

const ED25519_SIGNATURE_LENGTH: usize = 64;
pub const DEVICE_REFRESH_REPLAY_SALT_LENGTH: usize = 32;

#[derive(Clone, PartialEq, Eq)]
pub struct DeviceSignature([u8; ED25519_SIGNATURE_LENGTH]);

impl DeviceSignature {
    /// 从 Ed25519 签名字节创建不透明签名。
    ///
    /// # Errors
    ///
    /// 长度不是 64 字节时返回校验错误。
    pub fn new(bytes: Vec<u8>) -> Result<Self, DeviceProofValueError> {
        let bytes = <[u8; ED25519_SIGNATURE_LENGTH]>::try_from(bytes)
            .map_err(|_| DeviceProofValueError::InvalidSignature)?;
        Ok(Self(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; ED25519_SIGNATURE_LENGTH] {
        &self.0
    }
}

impl fmt::Debug for DeviceSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[已脱敏]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProofValueError {
    InvalidSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSessionRegistration {
    pub authorization_token_digest: SecretDigest,
    pub authorization_receipt_expires_at: UtcMillis,
    pub family: DeviceTokenFamily,
    pub access_token_id: DeviceAccessTokenId,
    pub access_token_digest: SecretDigest,
    pub access_token_expires_at: UtcMillis,
    pub refresh_token_id: DeviceRefreshTokenId,
    pub refresh_token_digest: SecretDigest,
    pub issued_at: UtcMillis,
}

/// 派生同一刷新尝试令牌对所用的服务端随机盐。
///
/// 盐本身不足以还原令牌，还需要只在请求里出现、从不落库的旧刷新令牌明文。
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceRefreshReplaySalt([u8; DEVICE_REFRESH_REPLAY_SALT_LENGTH]);

impl DeviceRefreshReplaySalt {
    pub const fn from_array(value: [u8; DEVICE_REFRESH_REPLAY_SALT_LENGTH]) -> Self {
        Self(value)
    }

    pub const fn as_bytes(&self) -> &[u8; DEVICE_REFRESH_REPLAY_SALT_LENGTH] {
        &self.0
    }
}

impl fmt::Debug for DeviceRefreshReplaySalt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[已脱敏]")
    }
}

/// 客户端生成的刷新尝试号，以及服务端为这次轮换抽取的派生盐。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRefreshReplay {
    pub attempt_id: DeviceRefreshAttemptId,
    pub salt: DeviceRefreshReplaySalt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTokenReplacement {
    pub access_token_id: DeviceAccessTokenId,
    pub access_token_digest: SecretDigest,
    pub access_token_expires_at: UtcMillis,
    pub refresh_token_id: DeviceRefreshTokenId,
    pub refresh_token_digest: SecretDigest,
    pub issued_at: UtcMillis,
    /// 旧客户端不带尝试号，轮换后无法重放；新客户端带尝试号，同一尝试可安全重试。
    pub replay: Option<DeviceRefreshReplay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedDeviceTokens {
    pub access_token: SecretValue,
    pub refresh_token: SecretValue,
}

/// 按「旧刷新令牌 + 尝试号 + 服务端盐」派生一次轮换的新令牌对。
///
/// 服务端只保存盐和新令牌摘要。同一尝试重试时重新派生即可得到同一对令牌，
/// 不需要保存令牌明文，也不需要新的服务端长期密钥。
pub trait DeviceRefreshTokenDerivation: Send + Sync {
    /// # Errors
    ///
    /// 操作系统安全随机源不可用时返回错误。
    fn replay_salt(&self) -> Result<DeviceRefreshReplaySalt, SecretGenerationFailure>;

    /// # Errors
    ///
    /// 派生结果无法构成合法敏感值时返回错误。
    fn derive(
        &self,
        refresh_token: &SecretValue,
        replay: &DeviceRefreshReplay,
    ) -> Result<DerivedDeviceTokens, SecretGenerationFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDeviceSession {
    pub account: PrincipalAccount,
    pub device: Device,
    pub family: DeviceTokenFamily,
    pub access_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRefreshContext {
    pub account: PrincipalAccount,
    pub device: Device,
    pub family: DeviceTokenFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceSecurityEvent {
    pub id: OutboxEventId,
    pub occurred_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRefreshOutcome {
    Rotated {
        session: Box<StoredDeviceSession>,
        refresh_token_expires_at: UtcMillis,
    },
    /// 同一刷新尝试已经轮换过，且它签发的令牌仍是当前令牌；调用方用原盐重新派生同一对令牌。
    Replayed {
        session: Box<StoredDeviceSession>,
        refresh_token_expires_at: UtcMillis,
        salt: DeviceRefreshReplaySalt,
        access_token_digest: SecretDigest,
        refresh_token_digest: SecretDigest,
    },
    ReuseDetected {
        device_id: DeviceId,
        principal_id: PrincipalId,
    },
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAgentMatrixDeviceRevocation {
    pub instance_id: AgentInstanceId,
    pub matrix_user_id: String,
    pub matrix_device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRevocationOutcome {
    Revoked(Vec<PendingAgentMatrixDeviceRevocation>),
    AlreadyRevoked(Vec<PendingAgentMatrixDeviceRevocation>),
    NotFound,
}

pub trait DeviceRegistrationTransaction: Send + Sync {
    fn register<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        device: &'a Device,
        session: &'a DeviceSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredDeviceSession>>;
}

pub trait DeviceSessionStore: Send + Sync {
    fn find_active_access<'a>(
        &'a self,
        access_token_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredDeviceSession>>>;

    fn find_refresh_context<'a>(
        &'a self,
        refresh_token_digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<DeviceRefreshContext>>>;

    fn rotate_refresh<'a>(
        &'a self,
        refresh_token_digest: &'a SecretDigest,
        replacement: &'a DeviceTokenReplacement,
        security_event: DeviceSecurityEvent,
    ) -> PortFuture<'a, RepositoryResult<DeviceRefreshOutcome>>;
}

pub trait DeviceProofNonceStore: Send + Sync {
    fn consume<'a>(
        &'a self,
        device_id: DeviceId,
        nonce_digest: &'a SecretDigest,
        consumed_at: UtcMillis,
        expires_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>>;
}

pub trait DeviceProofVerifier: Send + Sync {
    fn verify(
        &self,
        public_key: &DevicePublicSigningKey,
        signed_message: &[u8],
        signature: &DeviceSignature,
    ) -> bool;
}

pub trait DeviceRepository: Send + Sync {
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<Device>>>;
}

pub trait DeviceRevocationTransaction: Send + Sync {
    fn revoke(
        &self,
        principal_id: PrincipalId,
        device_id: DeviceId,
        security_event: DeviceSecurityEvent,
    ) -> PortFuture<'_, RepositoryResult<DeviceRevocationOutcome>>;
}
