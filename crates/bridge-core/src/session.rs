use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{Clock, PortFuture, SecretFactory, SecretValue},
};
use agent_room_domain::{
    ids::DeviceId,
    time::{DurationMillis, UtcMillis},
};
use tokio::sync::Mutex;

use crate::ports::{
    BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialState,
    ControlPlaneDeviceFailure, ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway,
    DeviceCredentialVault, DeviceRefreshAttemptIdFactory, DeviceSigningIdentityStore,
    RefreshBridgeDevice, StoredBridgeDeviceCredentials,
};

const REFRESH_DEVICE_PATH: &str = "/auth/devices/refresh";

const REFRESH_PROOF_OPERATIONS: ProofFailureOperations = ProofFailureOperations {
    load_key: "bridge.session.load_key",
    nonce: "bridge.session.nonce",
    payload: "bridge.session.proof",
    sign: "bridge.session.sign",
};

const CONTROL_PLANE_PROOF_OPERATIONS: ProofFailureOperations = ProofFailureOperations {
    load_key: "bridge.session.authorize_request.load_key",
    nonce: "bridge.session.authorize_request.nonce",
    payload: "bridge.session.authorize_request.proof",
    sign: "bridge.session.authorize_request.sign",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveBridgeSession {
    pub device_id: DeviceId,
    pub access_token: SecretValue,
    pub access_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedControlPlaneRequest {
    pub access_token: SecretValue,
    pub proof: DeviceRequestProof,
}

pub trait ControlPlaneRequestAuthorizer: Send + Sync {
    fn authorize<'a>(
        &'a self,
        method: &'a str,
        request_target: &'a str,
        body: &'a str,
    ) -> PortFuture<'a, BridgeSessionResult<AuthorizedControlPlaneRequest>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeSessionFailureKind {
    NotAuthorized,
    /// 刷新结果未知。待决状态和尝试号已保留，稍后会用同一尝试号安全重试，属于暂时不可用。
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
    refresh_attempts: Arc<dyn DeviceRefreshAttemptIdFactory>,
    policy: BridgeSessionPolicy,
    session_lock: Mutex<()>,
    refresh_outcomes: StdMutex<RefreshOutcomeLog>,
}

pub struct BridgeSessionDependencies {
    pub signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    pub control_plane: Arc<dyn ControlPlaneDeviceGateway>,
    pub credentials: Arc<dyn DeviceCredentialVault>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
    pub refresh_attempts: Arc<dyn DeviceRefreshAttemptIdFactory>,
}

/// 最近一次刷新请求的结论。排在它后面等锁的调用直接共享失败结论，
/// 不必各自再向控制面发一遍同一尝试。
#[derive(Clone, Copy, Default)]
struct RefreshOutcomeLog {
    completed: u64,
    last_failure: Option<BridgeSessionFailure>,
}

#[derive(Clone, Copy)]
struct ProofFailureOperations {
    load_key: &'static str,
    nonce: &'static str,
    payload: &'static str,
    sign: &'static str,
}

#[derive(Clone, Copy)]
struct ProofInput<'a> {
    device_id: DeviceId,
    credential: &'a SecretValue,
    issued_at: UtcMillis,
    method: &'a str,
    request_target: &'a str,
    body: &'a str,
}

impl BridgeSessionService {
    pub fn new(dependencies: BridgeSessionDependencies, policy: BridgeSessionPolicy) -> Self {
        Self {
            signing_identities: dependencies.signing_identities,
            control_plane: dependencies.control_plane,
            credentials: dependencies.credentials,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
            refresh_attempts: dependencies.refresh_attempts,
            policy,
            session_lock: Mutex::new(()),
            refresh_outcomes: StdMutex::new(RefreshOutcomeLog::default()),
        }
    }

    /// 返回可用的短期访问会话，必要时先完成刷新轮换。
    ///
    /// 刷新前先持久化带尝试号的待决状态。结果未知时保留它，之后的调用（包括重启后）
    /// 用同一尝试号对账：服务端要么这时才轮换，要么重放当时签发的同一对令牌。
    ///
    /// # Errors
    ///
    /// 未授权、刷新结果未知、控制平面不可用或 OS 安全存储失败时返回稳定错误。
    pub async fn active_session(&self) -> BridgeSessionResult<ActiveBridgeSession> {
        let observed = self.refresh_outcomes().completed;
        // 持锁覆盖凭据读取到刷新结果持久化；等待者必须重新读取，不能重用旧轮换令牌。
        let _session_guard = self.session_lock.lock().await;
        // 排队期间已有刷新以失败收场：直接共享这个结论，不再各自向控制面重发同一尝试；
        // 成功时照常往下读，拿到的就是新凭据。
        let latest = self.refresh_outcomes();
        if latest.completed != observed
            && let Some(failure) = latest.last_failure
        {
            return Err(failure);
        }
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

        let now = self.clock.now();
        if stored.refresh_token_expires_at <= now {
            self.clear_credentials("bridge.session.expired")?;
            return Err(failure(
                "bridge.session.expired",
                BridgeSessionFailureKind::NotAuthorized,
            ));
        }
        let attempt_id = match stored.state {
            BridgeCredentialState::Ready => {
                let refresh_at = now
                    .checked_add(self.policy.refresh_lead_time)
                    .map_err(|_| {
                        failure("bridge.session.time", BridgeSessionFailureKind::Internal)
                    })?;
                if stored.access_token_expires_at > refresh_at {
                    return Ok(active_session(&stored));
                }
                self.refresh_attempts.refresh_attempt_id()
            }
            BridgeCredentialState::RefreshPending {
                attempt_id: Some(attempt_id),
            } => attempt_id,
            // 旧版本留下的待决状态没有尝试号，只能换新尝试号重试。旧请求若其实已经提交，
            // 服务端会判为重用并拒绝，随后清除本机凭据、回到设备授权。
            BridgeCredentialState::RefreshPending { attempt_id: None } => {
                self.refresh_attempts.refresh_attempt_id()
            }
        };

        let proof = self.refresh_proof(&stored, now)?;
        let pending = BridgeCredentialState::RefreshPending {
            attempt_id: Some(attempt_id),
        };
        if stored.state != pending {
            stored.state = pending;
            self.credentials
                .replace(&stored)
                .map_err(|error| map_credential_failure("bridge.session.mark_pending", error))?;
        }
        let recorder = RefreshOutcomeRecorder {
            outcomes: &self.refresh_outcomes,
            finished: false,
        };
        let result = self
            .control_plane
            .refresh(RefreshBridgeDevice {
                refresh_token: stored.refresh_token.clone(),
                proof,
                attempt_id,
            })
            .await;
        let resolved = self.resolve_refresh(&stored, result);
        recorder.finish(&resolved);
        resolved
    }

    /// 丢弃本机设备会话凭据，下一次取会话时回到设备授权。
    ///
    /// 只在用户明确要求重新授权这台电脑时调用。设备签名密钥保留，重新注册后仍是同一台设备，
    /// 服务端会随注册撤销这台设备旧的 Token 族。
    ///
    /// # Errors
    ///
    /// OS 安全存储拒绝删除时返回稳定错误。
    pub async fn forget_device_session(&self) -> BridgeSessionResult<()> {
        let _session_guard = self.session_lock.lock().await;
        self.clear_credentials("bridge.session.forget")
    }

    fn refresh_outcomes(&self) -> RefreshOutcomeLog {
        *self
            .refresh_outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// 为控制面请求获取有效访问令牌，并对精确方法、目标和正文签名。
    ///
    /// # Errors
    ///
    /// 会话不可用、刷新失败、密钥不可用或请求证明不合法时返回稳定错误。
    pub async fn authorize_request(
        &self,
        method: &str,
        request_target: &str,
        body: &str,
    ) -> BridgeSessionResult<AuthorizedControlPlaneRequest> {
        let session = self.active_session().await?;
        let proof = self.signed_proof(
            ProofInput {
                device_id: session.device_id,
                credential: &session.access_token,
                issued_at: self.clock.now(),
                method,
                request_target,
                body,
            },
            CONTROL_PLANE_PROOF_OPERATIONS,
        )?;
        Ok(AuthorizedControlPlaneRequest {
            access_token: session.access_token,
            proof,
        })
    }

    fn refresh_proof(
        &self,
        stored: &StoredBridgeDeviceCredentials,
        now: UtcMillis,
    ) -> BridgeSessionResult<DeviceRequestProof> {
        self.signed_proof(
            ProofInput {
                device_id: stored.device_id,
                credential: &stored.refresh_token,
                issued_at: now,
                method: "POST",
                request_target: REFRESH_DEVICE_PATH,
                body: "",
            },
            REFRESH_PROOF_OPERATIONS,
        )
    }

    fn signed_proof(
        &self,
        input: ProofInput<'_>,
        operations: ProofFailureOperations,
    ) -> BridgeSessionResult<DeviceRequestProof> {
        let identity = self
            .signing_identities
            .load_or_create()
            .map_err(|error| map_credential_failure(operations.load_key, error))?;
        let nonce = self
            .secrets
            .generate()
            .map_err(|_| failure(operations.nonce, BridgeSessionFailureKind::Internal))?;
        let payload = DeviceRequestProofPayload::new(
            input.device_id,
            input.issued_at,
            nonce,
            input.method.to_owned(),
            input.request_target.to_owned(),
            self.secrets.digest(input.body),
        )
        .map_err(|_| failure(operations.payload, BridgeSessionFailureKind::Internal))?;
        let message = payload.signing_message(&self.secrets.digest(input.credential.expose()));
        let signature = identity
            .sign(&message)
            .map_err(|error| map_credential_failure(operations.sign, error))?;
        Ok(DeviceRequestProof::new(payload, signature))
    }

    fn resolve_refresh(
        &self,
        pending: &StoredBridgeDeviceCredentials,
        result: Result<
            agent_room_application::devices::DeviceCredentials,
            ControlPlaneDeviceFailure,
        >,
    ) -> BridgeSessionResult<ActiveBridgeSession> {
        let credentials = match result {
            Ok(credentials) => credentials,
            Err(error) => {
                let kind = match error.kind() {
                    // 服务端确认旧刷新令牌已不可用（撤销、过期或判为重用），只能重新授权。
                    ControlPlaneDeviceFailureKind::AuthenticationRejected
                    | ControlPlaneDeviceFailureKind::Conflict => {
                        self.clear_credentials("bridge.session.reject")?;
                        BridgeSessionFailureKind::NotAuthorized
                    }
                    // 其余失败都无法确认服务端是否已经轮换：保留待决状态和尝试号，稍后安全重试。
                    ControlPlaneDeviceFailureKind::UnknownCommit => {
                        BridgeSessionFailureKind::RefreshOutcomeUnknown
                    }
                    ControlPlaneDeviceFailureKind::DependencyUnavailable => {
                        BridgeSessionFailureKind::ControlPlaneUnavailable
                    }
                    ControlPlaneDeviceFailureKind::InvalidRequest
                    | ControlPlaneDeviceFailureKind::Internal => {
                        BridgeSessionFailureKind::InvalidControlPlaneResponse
                    }
                };
                return Err(failure("bridge.session.refresh", kind));
            }
        };
        if credentials.device.device_id != pending.device_id {
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
        self.credentials
            .replace(&replacement)
            .map_err(|error| map_credential_failure("bridge.session.persist_refresh", error))?;
        Ok(active_session(&replacement))
    }

    fn clear_credentials(&self, operation: &'static str) -> BridgeSessionResult<()> {
        self.credentials
            .clear()
            .map_err(|error| map_credential_failure(operation, error))
    }
}

/// 记录一次刷新请求的结论。请求在得到结论前被取消时，按结果未知记录。
struct RefreshOutcomeRecorder<'a> {
    outcomes: &'a StdMutex<RefreshOutcomeLog>,
    finished: bool,
}

impl RefreshOutcomeRecorder<'_> {
    fn finish(mut self, result: &BridgeSessionResult<ActiveBridgeSession>) {
        self.record(result.as_ref().err().copied());
        self.finished = true;
    }

    fn record(&self, failure: Option<BridgeSessionFailure>) {
        let mut outcomes = self.outcomes.lock().unwrap_or_else(PoisonError::into_inner);
        outcomes.completed = outcomes.completed.wrapping_add(1);
        outcomes.last_failure = failure;
    }
}

impl Drop for RefreshOutcomeRecorder<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.record(Some(failure(
                "bridge.session.refresh",
                BridgeSessionFailureKind::RefreshOutcomeUnknown,
            )));
        }
    }
}

impl ControlPlaneRequestAuthorizer for BridgeSessionService {
    fn authorize<'a>(
        &'a self,
        method: &'a str,
        request_target: &'a str,
        body: &'a str,
    ) -> PortFuture<'a, BridgeSessionResult<AuthorizedControlPlaneRequest>> {
        Box::pin(self.authorize_request(method, request_target, body))
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
