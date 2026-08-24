use std::sync::Arc;

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{Clock, SecretFactory, SecretValue},
};
use agent_room_domain::{
    ids::DeviceId,
    time::{DurationMillis, UtcMillis},
};

use crate::ports::{
    BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialState,
    ControlPlaneDeviceFailure, ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway,
    DeviceCredentialVault, DeviceSigningIdentityStore, RefreshBridgeDevice,
    StoredBridgeDeviceCredentials,
};

const REFRESH_DEVICE_PATH: &str = "/auth/devices/refresh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveBridgeSession {
    pub device_id: DeviceId,
    pub access_token: SecretValue,
    pub access_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSessionFailureKind {
    NotAuthorized,
    RefreshOutcomeUnknown,
    SecureStorageUnavailable,
    CorruptSecureStorage,
    ControlPlaneUnavailable,
    InvalidControlPlaneResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeSessionFailure {
    operation: &'static str,
    kind: BridgeSessionFailureKind,
}

impl BridgeSessionFailure {
    const fn new(operation: &'static str, kind: BridgeSessionFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> BridgeSessionFailureKind {
        self.kind
    }
}

pub type BridgeSessionResult<T> = Result<T, BridgeSessionFailure>;

pub struct BridgeSessionPolicy {
    refresh_lead_time: DurationMillis,
}

impl BridgeSessionPolicy {
    pub const fn new(refresh_lead_time: DurationMillis) -> Self {
        Self { refresh_lead_time }
    }
}

pub struct BridgeSessionService {
    signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    control_plane: Arc<dyn ControlPlaneDeviceGateway>,
    credentials: Arc<dyn DeviceCredentialVault>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
    policy: BridgeSessionPolicy,
}

pub struct BridgeSessionDependencies {
    pub signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    pub control_plane: Arc<dyn ControlPlaneDeviceGateway>,
    pub credentials: Arc<dyn DeviceCredentialVault>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
}

impl BridgeSessionService {
    pub fn new(dependencies: BridgeSessionDependencies, policy: BridgeSessionPolicy) -> Self {
        Self {
            signing_identities: dependencies.signing_identities,
            control_plane: dependencies.control_plane,
            credentials: dependencies.credentials,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
            policy,
        }
    }

    /// 返回可用的短期访问会话，必要时先完成一次不可重试的刷新轮换。
    ///
    /// # Errors
    ///
    /// 未授权、刷新结果未知、控制平面不可用或 OS 安全存储失败时返回稳定错误。
    pub async fn active_session(&self) -> BridgeSessionResult<ActiveBridgeSession> {
        let mut stored = self
            .credentials
            .load()
            .map_err(|error| map_credential_failure("bridge.session.load", error))?
            .ok_or_else(|| {
                failure(
                    "bridge.session.load",
                    BridgeSessionFailureKind::NotAuthorized,
                )
            })?;
        if stored.state == BridgeCredentialState::RefreshPending {
            return Err(failure(
                "bridge.session.load",
                BridgeSessionFailureKind::RefreshOutcomeUnknown,
            ));
        }

        let now = self.clock.now();
        if stored.refresh_token_expires_at <= now {
            self.clear_credentials("bridge.session.expired")?;
            return Err(failure(
                "bridge.session.expired",
                BridgeSessionFailureKind::NotAuthorized,
            ));
        }
        let refresh_at = now
            .checked_add(self.policy.refresh_lead_time)
            .map_err(|_| failure("bridge.session.time", BridgeSessionFailureKind::Internal))?;
        if stored.access_token_expires_at > refresh_at {
            return Ok(active_session(&stored));
        }

        let proof = self.refresh_proof(&stored, now)?;
        stored.state = BridgeCredentialState::RefreshPending;
        self.credentials
            .replace(&stored)
            .map_err(|error| map_credential_failure("bridge.session.mark_pending", error))?;
        let result = self
            .control_plane
            .refresh(RefreshBridgeDevice {
                refresh_token: stored.refresh_token.clone(),
                proof,
            })
            .await;
        self.resolve_refresh(stored, result)
    }

    fn refresh_proof(
        &self,
        stored: &StoredBridgeDeviceCredentials,
        now: UtcMillis,
    ) -> BridgeSessionResult<DeviceRequestProof> {
        let identity = self
            .signing_identities
            .load_or_create()
            .map_err(|error| map_credential_failure("bridge.session.load_key", error))?;
        let nonce = self
            .secrets
            .generate()
            .map_err(|_| failure("bridge.session.nonce", BridgeSessionFailureKind::Internal))?;
        let payload = DeviceRequestProofPayload::new(
            stored.device_id,
            now,
            nonce,
            "POST".to_owned(),
            REFRESH_DEVICE_PATH.to_owned(),
            self.secrets.digest(""),
        )
        .map_err(|_| failure("bridge.session.proof", BridgeSessionFailureKind::Internal))?;
        let message = payload.signing_message(&self.secrets.digest(stored.refresh_token.expose()));
        let signature = identity
            .sign(&message)
            .map_err(|error| map_credential_failure("bridge.session.sign", error))?;
        Ok(DeviceRequestProof::new(payload, signature))
    }

    fn resolve_refresh(
        &self,
        mut previous: StoredBridgeDeviceCredentials,
        result: Result<
            agent_room_application::devices::DeviceCredentials,
            ControlPlaneDeviceFailure,
        >,
    ) -> BridgeSessionResult<ActiveBridgeSession> {
        match result {
            Ok(credentials) => {
                if credentials.device.device_id != previous.device_id {
                    return Err(failure(
                        "bridge.session.refresh",
                        BridgeSessionFailureKind::InvalidControlPlaneResponse,
                    ));
                }
                let replacement = StoredBridgeDeviceCredentials {
                    state: BridgeCredentialState::Ready,
                    device_id: credentials.device.device_id,
                    access_token: credentials.access_token,
                    access_token_expires_at: credentials.device.access_token_expires_at,
                    refresh_token: credentials.refresh_token,
                    refresh_token_expires_at: credentials.refresh_token_expires_at,
                };
                self.credentials.replace(&replacement).map_err(|error| {
                    map_credential_failure("bridge.session.persist_refresh", error)
                })?;
                Ok(active_session(&replacement))
            }
            Err(error) => match error.kind() {
                ControlPlaneDeviceFailureKind::UnknownCommit => Err(failure(
                    "bridge.session.refresh",
                    BridgeSessionFailureKind::RefreshOutcomeUnknown,
                )),
                ControlPlaneDeviceFailureKind::AuthenticationRejected
                | ControlPlaneDeviceFailureKind::Conflict => {
                    self.clear_credentials("bridge.session.reject")?;
                    Err(failure(
                        "bridge.session.refresh",
                        BridgeSessionFailureKind::NotAuthorized,
                    ))
                }
                ControlPlaneDeviceFailureKind::DependencyUnavailable => {
                    previous.state = BridgeCredentialState::Ready;
                    self.credentials.replace(&previous).map_err(|failure| {
                        map_credential_failure("bridge.session.restore", failure)
                    })?;
                    Err(failure(
                        "bridge.session.refresh",
                        BridgeSessionFailureKind::ControlPlaneUnavailable,
                    ))
                }
                ControlPlaneDeviceFailureKind::InvalidRequest
                | ControlPlaneDeviceFailureKind::Internal => {
                    previous.state = BridgeCredentialState::Ready;
                    self.credentials.replace(&previous).map_err(|failure| {
                        map_credential_failure("bridge.session.restore", failure)
                    })?;
                    Err(failure(
                        "bridge.session.refresh",
                        BridgeSessionFailureKind::InvalidControlPlaneResponse,
                    ))
                }
            },
        }
    }

    fn clear_credentials(&self, operation: &'static str) -> BridgeSessionResult<()> {
        self.credentials
            .clear()
            .map_err(|error| map_credential_failure(operation, error))
    }
}

fn active_session(stored: &StoredBridgeDeviceCredentials) -> ActiveBridgeSession {
    ActiveBridgeSession {
        device_id: stored.device_id,
        access_token: stored.access_token.clone(),
        access_token_expires_at: stored.access_token_expires_at,
    }
}

const fn map_credential_failure(
    operation: &'static str,
    error: BridgeCredentialFailure,
) -> BridgeSessionFailure {
    let kind = match error.kind() {
        BridgeCredentialFailureKind::Unavailable => {
            BridgeSessionFailureKind::SecureStorageUnavailable
        }
        BridgeCredentialFailureKind::Corrupt => BridgeSessionFailureKind::CorruptSecureStorage,
    };
    failure(operation, kind)
}

const fn failure(operation: &'static str, kind: BridgeSessionFailureKind) -> BridgeSessionFailure {
    BridgeSessionFailure::new(operation, kind)
}
