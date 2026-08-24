use std::sync::Arc;

use agent_room_application::{
    devices::{DeviceCredentials, DeviceRequestProof},
    ports::{
        DeviceSignature, MatrixEventId, MatrixResult, MatrixRoomId, MatrixStateEvent, PortFuture,
        SecretValue,
    },
};
use agent_room_domain::{
    devices::{DevicePlatform, DevicePublicSigningKey},
    ids::DeviceId,
    time::UtcMillis,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCredentialFailureKind {
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeCredentialFailure {
    kind: BridgeCredentialFailureKind,
}

impl BridgeCredentialFailure {
    pub const fn new(kind: BridgeCredentialFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> BridgeCredentialFailureKind {
        self.kind
    }
}

pub type BridgeCredentialResult<T> = Result<T, BridgeCredentialFailure>;

pub trait DeviceSigningIdentity: Send + Sync {
    /// # Errors
    ///
    /// 密钥损坏或安全存储不可用时返回错误。
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey>;

    /// # Errors
    ///
    /// 私钥不可用或密码学签名失败时返回错误。
    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature>;
}

pub trait DeviceSigningIdentityStore: Send + Sync {
    /// 从 OS 安全存储加载现有密钥，不存在时生成并持久化新密钥。
    ///
    /// # Errors
    ///
    /// 安全存储不可用、密钥损坏或密码学随机源不可用时返回错误。
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBridgeDeviceCredentials {
    pub state: BridgeCredentialState,
    pub device_id: DeviceId,
    pub access_token: SecretValue,
    pub access_token_expires_at: UtcMillis,
    pub refresh_token: SecretValue,
    pub refresh_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCredentialState {
    Ready,
    RefreshPending,
}

pub trait DeviceCredentialVault: Send + Sync {
    /// # Errors
    ///
    /// 安全存储不可用或已保存凭据损坏时返回错误。
    fn load(&self) -> BridgeCredentialResult<Option<StoredBridgeDeviceCredentials>>;

    /// 原子替换当前设备会话；不得先删除旧值再逐项写入。
    ///
    /// # Errors
    ///
    /// 安全存储拒绝写入时返回错误。
    fn replace(&self, credentials: &StoredBridgeDeviceCredentials) -> BridgeCredentialResult<()>;

    /// # Errors
    ///
    /// 安全存储拒绝删除时返回错误。
    fn clear(&self) -> BridgeCredentialResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterBridgeDevice {
    pub oidc_assertion: SecretValue,
    pub label: String,
    pub platform: DevicePlatform,
    pub public_signing_key: DevicePublicSigningKey,
    pub possession_signature: DeviceSignature,
    pub import_display_name: bool,
    pub import_locale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshBridgeDevice {
    pub refresh_token: SecretValue,
    pub proof: DeviceRequestProof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneDeviceFailureKind {
    InvalidRequest,
    AuthenticationRejected,
    Conflict,
    DependencyUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlaneDeviceFailure {
    kind: ControlPlaneDeviceFailureKind,
}

impl ControlPlaneDeviceFailure {
    pub const fn new(kind: ControlPlaneDeviceFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> ControlPlaneDeviceFailureKind {
        self.kind
    }
}

pub type ControlPlaneDeviceResult<T> = Result<T, ControlPlaneDeviceFailure>;

pub trait ControlPlaneDeviceGateway: Send + Sync {
    fn register(
        &self,
        request: RegisterBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>>;

    fn refresh(
        &self,
        request: RefreshBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>>;
}

/// 将状态发布用例与完整 Matrix 会话能力隔离开的最小端口。
pub trait AgentStatusStatePublisher: Send + Sync {
    fn publish<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>>;
}

pub trait StatusEventIdentifierFactory: Send + Sync {
    fn event_id(&self) -> uuid::Uuid;
    fn correlation_id(&self) -> uuid::Uuid;
}
