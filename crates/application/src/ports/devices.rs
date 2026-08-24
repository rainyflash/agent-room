use std::fmt;

use agent_room_domain::{
    devices::{Device, DevicePublicSigningKey, DeviceTokenFamily},
    ids::{DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, OutboxEventId, PrincipalId},
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::{PortFuture, PrincipalAccount, PrincipalRegistration, SecretDigest};

const ED25519_SIGNATURE_LENGTH: usize = 64;

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
    pub family: DeviceTokenFamily,
    pub access_token_id: DeviceAccessTokenId,
    pub access_token_digest: SecretDigest,
    pub access_token_expires_at: UtcMillis,
    pub refresh_token_id: DeviceRefreshTokenId,
    pub refresh_token_digest: SecretDigest,
    pub issued_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTokenReplacement {
    pub access_token_id: DeviceAccessTokenId,
    pub access_token_digest: SecretDigest,
    pub access_token_expires_at: UtcMillis,
    pub refresh_token_id: DeviceRefreshTokenId,
    pub refresh_token_digest: SecretDigest,
    pub issued_at: UtcMillis,
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
    ReuseDetected {
        device_id: DeviceId,
        principal_id: PrincipalId,
    },
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRevocationOutcome {
    Revoked,
    AlreadyRevoked,
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
