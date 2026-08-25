use agent_room_domain::{
    agents::{
        AdapterBinding, Agent, AgentInstance, AgentInstancePublicSigningKey, AgentMemberships,
        AgentRole, AgentVisibility,
    },
    devices::{DevicePlatform, DeviceTrustState},
    ids::{
        AgentCreationRequestId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId,
        ContentId, DeviceId, PrincipalId,
    },
    time::UtcMillis,
};
use serde_json::{Map, Value};

use crate::persistence::RepositoryResult;

use super::{DeviceSignature, OutboxMessage, PortFuture, SecretDigest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistration {
    pub agent: Agent,
    pub owner_id: PrincipalId,
    pub matrix_user_id: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub avatar_content_id: Option<ContentId>,
    pub visibility: AgentVisibility,
    pub registered_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAgent {
    pub agent: Agent,
    pub matrix_user_id: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub avatar_content_id: Option<ContentId>,
    pub visibility: AgentVisibility,
    pub registered_at: UtcMillis,
}

impl From<&AgentRegistration> for RegisteredAgent {
    fn from(registration: &AgentRegistration) -> Self {
        Self {
            agent: registration.agent.clone(),
            matrix_user_id: registration.matrix_user_id.clone(),
            slug: registration.slug.clone(),
            display_name: registration.display_name.clone(),
            description: registration.description.clone(),
            avatar_content_id: registration.avatar_content_id,
            visibility: registration.visibility,
            registered_at: registration.registered_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCreationClaim {
    pub request_id: AgentCreationRequestId,
    pub owner_id: PrincipalId,
    pub proposed_agent_id: AgentId,
    pub request_fingerprint: SecretDigest,
    pub reserved_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCreationReservation {
    Reserved { agent_id: AgentId },
    Completed(RegisteredAgent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstanceRegistration {
    pub request_id: AgentInstanceRegistrationRequestId,
    pub principal_id: PrincipalId,
    pub device_id: DeviceId,
    pub request_fingerprint: SecretDigest,
    pub binding: AdapterBinding,
    pub binding_configuration: Map<String, Value>,
    pub instance: AgentInstance,
    pub registered_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAgentInstanceRegistration {
    pub binding: AdapterBinding,
    pub instance: AgentInstance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstanceVerificationRecord {
    pub instance_id: AgentInstanceId,
    pub agent_id: AgentId,
    pub public_signing_key: AgentInstancePublicSigningKey,
    pub registered_at: UtcMillis,
    pub invalidated_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstanceManagementRecord {
    pub instance: AgentInstance,
    pub agent_matrix_user_id: String,
    pub agent_display_name: String,
    pub agent_avatar_content_id: Option<ContentId>,
    pub adapter_type: String,
    pub capability_version: String,
    pub device_label: String,
    pub device_platform: DevicePlatform,
    pub device_trust_state: DeviceTrustState,
    pub created_at: UtcMillis,
    pub last_seen_at: Option<UtcMillis>,
    pub revoked_at: Option<UtcMillis>,
    pub matrix_device_revoked_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentInstanceRevocationOutcome {
    Revoked(AgentInstanceManagementRecord),
    AlreadyRevoked(AgentInstanceManagementRecord),
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentMembershipChange {
    pub agent_id: AgentId,
    pub actor_id: PrincipalId,
    pub principal_id: PrincipalId,
    pub role: Option<AgentRole>,
    pub changed_at: UtcMillis,
}

pub trait AgentRepository: Send + Sync {
    fn find(&self, id: AgentId) -> PortFuture<'_, RepositoryResult<Option<Agent>>>;

    fn find_registration(
        &self,
        id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<RegisteredAgent>>>;

    fn create<'a>(
        &'a self,
        registration: &'a AgentRegistration,
    ) -> PortFuture<'a, RepositoryResult<Agent>>;

    fn save<'a>(&'a self, agent: &'a Agent) -> PortFuture<'a, RepositoryResult<Agent>>;
}

/// 为跨 Matrix 和 `PostgreSQL` 的 Agent 创建流程保留稳定业务标识。
pub trait AgentCreationWorkflow: Send + Sync {
    fn reserve<'a>(
        &'a self,
        claim: &'a AgentCreationClaim,
    ) -> PortFuture<'a, RepositoryResult<AgentCreationReservation>>;

    fn complete_with_event<'a>(
        &'a self,
        request_id: AgentCreationRequestId,
        request_fingerprint: &'a SecretDigest,
        registration: &'a AgentRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<Agent>>;
}

pub trait AgentMembershipRepository: Send + Sync {
    fn find_memberships(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentMemberships>>>;
}

pub trait AgentMembershipTransaction: Send + Sync {
    fn apply_change<'a>(
        &'a self,
        change: &'a AgentMembershipChange,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentMemberships>>;
}

pub trait AgentInstanceRegistrationTransaction: Send + Sync {
    fn register_with_event<'a>(
        &'a self,
        registration: &'a AgentInstanceRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<StoredAgentInstanceRegistration>>;
}

pub trait AgentInstanceVerificationRepository: Send + Sync {
    fn find_verification_record(
        &self,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceVerificationRecord>>>;
}

pub trait AgentInstanceManagementRepository: Send + Sync {
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentInstanceManagementRecord>>>;
}

pub trait AgentInstanceRevocationTransaction: Send + Sync {
    fn revoke<'a>(
        &'a self,
        principal_id: PrincipalId,
        instance_id: AgentInstanceId,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentInstanceRevocationOutcome>>;
}

pub trait AgentInstanceMatrixCleanupStore: Send + Sync {
    fn mark_matrix_device_revoked(
        &self,
        instance_id: AgentInstanceId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;
}

pub trait AgentInstanceSignatureVerifier: Send + Sync {
    fn verify(
        &self,
        public_key: &AgentInstancePublicSigningKey,
        signed_message: &[u8],
        signature: &DeviceSignature,
    ) -> bool;
}

/// 持久化 Agent 注册及其领域事件的单一事务边界。
pub trait AgentRegistrationTransaction: Send + Sync {
    fn create_with_event<'a>(
        &'a self,
        registration: &'a AgentRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<Agent>>;
}
