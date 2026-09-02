use std::sync::Arc;

use agent_room_application::ports::{MatrixSession, PortFuture};
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    ids::{AdapterBindingId, AgentId, AgentInstanceRegistrationRequestId},
};
use uuid::Version;

use crate::{
    agent_identity::BridgeAgentIdentity,
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
        DeviceSigningIdentityStore,
    },
};

const MAX_ADAPTER_TYPE_LENGTH: usize = 128;
const MAX_CAPABILITY_VERSION_LENGTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntimeRegistrationIntent {
    request_id: AgentInstanceRegistrationRequestId,
    agent_id: AgentId,
    adapter_type: String,
    capability_version: String,
    public_signing_key: AgentInstancePublicSigningKey,
}

impl AgentRuntimeRegistrationIntent {
    /// 创建一份可安全重试的实例登记意图。
    ///
    /// # Errors
    ///
    /// 请求标识不是 UUIDv7，或适配器元数据越界时返回无效配置。
    pub fn new(
        request_id: AgentInstanceRegistrationRequestId,
        agent_id: AgentId,
        adapter_type: impl Into<String>,
        capability_version: impl Into<String>,
        public_signing_key: AgentInstancePublicSigningKey,
    ) -> AgentRuntimeSessionResult<Self> {
        let adapter_type = adapter_type.into();
        let capability_version = capability_version.into();
        if request_id.as_uuid().get_version() != Some(Version::SortRand)
            || !valid_text(&adapter_type, MAX_ADAPTER_TYPE_LENGTH)
            || !valid_text(&capability_version, MAX_CAPABILITY_VERSION_LENGTH)
        {
            return Err(failure(
                "bridge.agent_runtime.intent",
                AgentRuntimeSessionFailureKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            request_id,
            agent_id,
            adapter_type,
            capability_version,
            public_signing_key,
        })
    }

    pub const fn request_id(&self) -> AgentInstanceRegistrationRequestId {
        self.request_id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub fn adapter_type(&self) -> &str {
        &self.adapter_type
    }

    pub fn capability_version(&self) -> &str {
        &self.capability_version
    }

    pub const fn public_signing_key(&self) -> &AgentInstancePublicSigningKey {
        &self.public_signing_key
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAgentRuntime {
    identity: BridgeAgentIdentity,
    adapter_binding_id: AdapterBindingId,
    matrix_session: MatrixSession,
}

impl RegisteredAgentRuntime {
    /// 创建控制面已登记的 Agent 运行时。
    ///
    /// # Errors
    ///
    /// Agent 身份与 Matrix 会话身份不一致时拒绝构造。
    pub fn new(
        identity: BridgeAgentIdentity,
        adapter_binding_id: AdapterBindingId,
        matrix_session: MatrixSession,
    ) -> AgentRuntimeSessionResult<Self> {
        if identity.matrix_user_id() != matrix_session.metadata().user_id() {
            return Err(failure(
                "bridge.agent_runtime.response",
                AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse,
            ));
        }
        Ok(Self {
            identity,
            adapter_binding_id,
            matrix_session,
        })
    }

    pub const fn identity(&self) -> &BridgeAgentIdentity {
        &self.identity
    }

    pub const fn adapter_binding_id(&self) -> AdapterBindingId {
        self.adapter_binding_id
    }

    pub const fn matrix_session(&self) -> &MatrixSession {
        &self.matrix_session
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredAgentRuntimeCredentials {
    RegistrationPending(AgentRuntimeRegistrationIntent),
    Ready {
        intent: AgentRuntimeRegistrationIntent,
        runtime: Box<RegisteredAgentRuntime>,
    },
}

pub trait AgentRuntimeCredentialVault: Send + Sync {
    /// # Errors
    ///
    /// 安全存储不可用或凭据损坏时返回错误。
    fn load(&self) -> BridgeCredentialResult<Option<StoredAgentRuntimeCredentials>>;

    /// # Errors
    ///
    /// 安全存储拒绝原子替换时返回错误。
    fn replace(&self, credentials: &StoredAgentRuntimeCredentials) -> BridgeCredentialResult<()>;

    /// # Errors
    ///
    /// 安全存储拒绝删除时返回错误。
    fn clear(&self) -> BridgeCredentialResult<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneAgentRuntimeFailureKind {
    InvalidRequest,
    AuthenticationRejected,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    UnknownCommit,
    InvalidResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlaneAgentRuntimeFailure {
    kind: ControlPlaneAgentRuntimeFailureKind,
}

impl ControlPlaneAgentRuntimeFailure {
    pub const fn new(kind: ControlPlaneAgentRuntimeFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> ControlPlaneAgentRuntimeFailureKind {
        self.kind
    }
}

pub type ControlPlaneAgentRuntimeResult<T> = Result<T, ControlPlaneAgentRuntimeFailure>;

pub trait ControlPlaneAgentRuntimeGateway: Send + Sync {
    fn register<'a>(
        &'a self,
        intent: &'a AgentRuntimeRegistrationIntent,
    ) -> PortFuture<'a, ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime>>;
}

pub trait AgentRuntimeRequestIdFactory: Send + Sync {
    fn registration_request_id(&self) -> AgentInstanceRegistrationRequestId;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntimeSessionConfig {
    agent_id: AgentId,
    adapter_type: String,
    capability_version: String,
}

impl AgentRuntimeSessionConfig {
    /// 创建稳定的 Agent 适配器绑定配置。
    ///
    /// # Errors
    ///
    /// 适配器名称或能力版本为空、超长或含控制字符时返回错误。
    pub fn new(
        agent_id: AgentId,
        adapter_type: impl Into<String>,
        capability_version: impl Into<String>,
    ) -> AgentRuntimeSessionResult<Self> {
        let adapter_type = adapter_type.into();
        let capability_version = capability_version.into();
        if !valid_text(&adapter_type, MAX_ADAPTER_TYPE_LENGTH)
            || !valid_text(&capability_version, MAX_CAPABILITY_VERSION_LENGTH)
        {
            return Err(failure(
                "bridge.agent_runtime.config",
                AgentRuntimeSessionFailureKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            agent_id,
            adapter_type,
            capability_version,
        })
    }
}

pub struct AgentRuntimeSessionService {
    signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    control_plane: Arc<dyn ControlPlaneAgentRuntimeGateway>,
    credentials: Arc<dyn AgentRuntimeCredentialVault>,
    identifiers: Arc<dyn AgentRuntimeRequestIdFactory>,
}

pub struct AgentRuntimeSessionDependencies {
    pub signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    pub control_plane: Arc<dyn ControlPlaneAgentRuntimeGateway>,
    pub credentials: Arc<dyn AgentRuntimeCredentialVault>,
    pub identifiers: Arc<dyn AgentRuntimeRequestIdFactory>,
}

impl AgentRuntimeSessionService {
    pub fn new(dependencies: AgentRuntimeSessionDependencies) -> Self {
        Self {
            signing_identities: dependencies.signing_identities,
            control_plane: dependencies.control_plane,
            credentials: dependencies.credentials,
            identifiers: dependencies.identifiers,
        }
    }

    /// 恢复或幂等登记当前 Agent 实例，并返回独立 Matrix Device 会话。
    ///
    /// # Errors
    ///
    /// 安全存储、签名身份、配置一致性或控制面登记失败时返回稳定错误。
    pub async fn ensure_session(
        &self,
        config: &AgentRuntimeSessionConfig,
    ) -> AgentRuntimeSessionResult<RegisteredAgentRuntime> {
        let public_signing_key = self.load_public_signing_key()?;

        let intent = match self
            .credentials
            .load()
            .map_err(|error| map_credential_failure("bridge.agent_runtime.load", error))?
        {
            Some(StoredAgentRuntimeCredentials::Ready { intent, runtime }) => {
                ensure_compatible(config, &intent, &public_signing_key)?;
                validate_registration(&intent, &runtime)?;
                return Ok(*runtime);
            }
            Some(StoredAgentRuntimeCredentials::RegistrationPending(intent)) => {
                ensure_compatible(config, &intent, &public_signing_key)?;
                intent
            }
            None => {
                let intent = AgentRuntimeRegistrationIntent::new(
                    self.identifiers.registration_request_id(),
                    config.agent_id,
                    config.adapter_type.clone(),
                    config.capability_version.clone(),
                    public_signing_key,
                )?;
                self.credentials
                    .replace(&StoredAgentRuntimeCredentials::RegistrationPending(
                        intent.clone(),
                    ))
                    .map_err(|error| {
                        map_credential_failure("bridge.agent_runtime.persist_intent", error)
                    })?;
                intent
            }
        };

        let runtime = self
            .control_plane
            .register(&intent)
            .await
            .map_err(|error| map_control_plane_failure("bridge.agent_runtime.register", error))?;
        validate_registration(&intent, &runtime)?;
        self.credentials
            .replace(&StoredAgentRuntimeCredentials::Ready {
                intent,
                runtime: Box::new(runtime.clone()),
            })
            .map_err(|error| {
                map_credential_failure("bridge.agent_runtime.persist_session", error)
            })?;
        Ok(runtime)
    }

    /// 重放已持久化的登记意图，并原子替换同一实例的 Matrix 设备会话。
    ///
    /// 该操作只允许轮换访问凭据；Agent、实例、适配器绑定、Matrix 用户和设备标识
    /// 任一发生变化都会拒绝写入，避免恢复流程静默劫持身份。
    ///
    /// # Errors
    ///
    /// 尚无就绪登记、配置漂移、控制面轮换失败或安全存储不可用时返回稳定错误。
    pub async fn recover_matrix_session(
        &self,
        config: &AgentRuntimeSessionConfig,
    ) -> AgentRuntimeSessionResult<RegisteredAgentRuntime> {
        let public_signing_key = self.load_public_signing_key()?;
        let (intent, current) = match self
            .credentials
            .load()
            .map_err(|error| map_credential_failure("bridge.agent_runtime.load", error))?
        {
            Some(StoredAgentRuntimeCredentials::Ready { intent, runtime }) => (intent, *runtime),
            Some(StoredAgentRuntimeCredentials::RegistrationPending(_)) | None => {
                return Err(failure(
                    "bridge.agent_runtime.recover_matrix_session",
                    AgentRuntimeSessionFailureKind::NotFound,
                ));
            }
        };
        ensure_compatible(config, &intent, &public_signing_key)?;
        validate_registration(&intent, &current)?;

        let recovered = self
            .control_plane
            .register(&intent)
            .await
            .map_err(|error| {
                map_control_plane_failure("bridge.agent_runtime.recover_matrix_session", error)
            })?;
        validate_registration(&intent, &recovered)?;
        validate_recovered_identity(&current, &recovered)?;
        self.credentials
            .replace(&StoredAgentRuntimeCredentials::Ready {
                intent,
                runtime: Box::new(recovered.clone()),
            })
            .map_err(|error| {
                map_credential_failure("bridge.agent_runtime.persist_recovery", error)
            })?;
        Ok(recovered)
    }

    fn load_public_signing_key(&self) -> AgentRuntimeSessionResult<AgentInstancePublicSigningKey> {
        let signer = self
            .signing_identities
            .load_or_create()
            .map_err(|error| map_credential_failure("bridge.agent_runtime.load_key", error))?;
        let device_public_key = signer
            .public_key()
            .map_err(|error| map_credential_failure("bridge.agent_runtime.public_key", error))?;
        AgentInstancePublicSigningKey::new(device_public_key.as_bytes().to_vec()).map_err(|_| {
            failure(
                "bridge.agent_runtime.public_key",
                AgentRuntimeSessionFailureKind::Internal,
            )
        })
    }
}

fn ensure_compatible(
    config: &AgentRuntimeSessionConfig,
    intent: &AgentRuntimeRegistrationIntent,
    public_signing_key: &AgentInstancePublicSigningKey,
) -> AgentRuntimeSessionResult<()> {
    if intent.agent_id != config.agent_id
        || intent.adapter_type != config.adapter_type
        || intent.capability_version != config.capability_version
        || &intent.public_signing_key != public_signing_key
    {
        return Err(failure(
            "bridge.agent_runtime.configuration_changed",
            AgentRuntimeSessionFailureKind::ConfigurationConflict,
        ));
    }
    Ok(())
}

fn validate_registration(
    intent: &AgentRuntimeRegistrationIntent,
    runtime: &RegisteredAgentRuntime,
) -> AgentRuntimeSessionResult<()> {
    if runtime.identity.agent_id() != intent.agent_id
        || runtime.identity.matrix_user_id() != runtime.matrix_session.metadata().user_id()
    {
        return Err(failure(
            "bridge.agent_runtime.validate_response",
            AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse,
        ));
    }
    Ok(())
}

fn validate_recovered_identity(
    current: &RegisteredAgentRuntime,
    recovered: &RegisteredAgentRuntime,
) -> AgentRuntimeSessionResult<()> {
    let current_session = current.matrix_session().metadata();
    let recovered_session = recovered.matrix_session().metadata();
    if current.identity().agent_id() != recovered.identity().agent_id()
        || current.identity().agent_instance_id() != recovered.identity().agent_instance_id()
        || current.adapter_binding_id() != recovered.adapter_binding_id()
        || current_session.user_id() != recovered_session.user_id()
        || current_session.device_id() != recovered_session.device_id()
    {
        return Err(failure(
            "bridge.agent_runtime.validate_recovery",
            AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRuntimeSessionFailureKind {
    InvalidConfiguration,
    ConfigurationConflict,
    NotAuthorized,
    Forbidden,
    NotFound,
    Conflict,
    ControlPlaneUnavailable,
    RegistrationOutcomeUnknown,
    InvalidControlPlaneResponse,
    SecureStorageUnavailable,
    CorruptSecureStorage,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRuntimeSessionFailure {
    operation: &'static str,
    kind: AgentRuntimeSessionFailureKind,
}

impl AgentRuntimeSessionFailure {
    pub const fn new(operation: &'static str, kind: AgentRuntimeSessionFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentRuntimeSessionFailureKind {
        self.kind
    }
}

pub type AgentRuntimeSessionResult<T> = Result<T, AgentRuntimeSessionFailure>;

fn map_credential_failure(
    operation: &'static str,
    failure: BridgeCredentialFailure,
) -> AgentRuntimeSessionFailure {
    let kind = match failure.kind() {
        BridgeCredentialFailureKind::Unavailable => {
            AgentRuntimeSessionFailureKind::SecureStorageUnavailable
        }
        BridgeCredentialFailureKind::Corrupt => {
            AgentRuntimeSessionFailureKind::CorruptSecureStorage
        }
    };
    AgentRuntimeSessionFailure::new(operation, kind)
}

fn map_control_plane_failure(
    operation: &'static str,
    failure: ControlPlaneAgentRuntimeFailure,
) -> AgentRuntimeSessionFailure {
    let kind = match failure.kind() {
        ControlPlaneAgentRuntimeFailureKind::InvalidRequest => {
            AgentRuntimeSessionFailureKind::InvalidConfiguration
        }
        ControlPlaneAgentRuntimeFailureKind::AuthenticationRejected => {
            AgentRuntimeSessionFailureKind::NotAuthorized
        }
        ControlPlaneAgentRuntimeFailureKind::Forbidden => AgentRuntimeSessionFailureKind::Forbidden,
        ControlPlaneAgentRuntimeFailureKind::NotFound => AgentRuntimeSessionFailureKind::NotFound,
        ControlPlaneAgentRuntimeFailureKind::Conflict => AgentRuntimeSessionFailureKind::Conflict,
        ControlPlaneAgentRuntimeFailureKind::Unavailable => {
            AgentRuntimeSessionFailureKind::ControlPlaneUnavailable
        }
        ControlPlaneAgentRuntimeFailureKind::UnknownCommit => {
            AgentRuntimeSessionFailureKind::RegistrationOutcomeUnknown
        }
        ControlPlaneAgentRuntimeFailureKind::InvalidResponse => {
            AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse
        }
        ControlPlaneAgentRuntimeFailureKind::Internal => AgentRuntimeSessionFailureKind::Internal,
    };
    AgentRuntimeSessionFailure::new(operation, kind)
}

const fn failure(
    operation: &'static str,
    kind: AgentRuntimeSessionFailureKind,
) -> AgentRuntimeSessionFailure {
    AgentRuntimeSessionFailure::new(operation, kind)
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
