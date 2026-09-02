use std::{fmt::Write as _, sync::Arc};

use agent_room_domain::{
    DomainError,
    agents::{
        AdapterBinding, AdapterSubjectHash, Agent, AgentInstance, AgentInstancePublicSigningKey,
        AgentMatrixDeviceId, AgentRole, AgentVisibility,
    },
    ids::{
        AgentCreationRequestId, AgentId, AgentInstanceRegistrationRequestId, ContentId, PrincipalId,
    },
};
use serde_json::{Map, Value};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentCreationClaim, AgentCreationReservation, AgentCreationWorkflow,
        AgentInstanceRegistration, AgentInstanceRegistrationTransaction, AgentMembershipChange,
        AgentMembershipRepository, AgentMembershipTransaction, AgentRegistration, AgentRepository,
        Clock, IdentifierFactory, MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner,
        MatrixAgentLocalpart, MatrixAgentUserRegistration, MatrixDeviceId, MatrixFailureKind,
        MatrixSession, MatrixUserId, OutboxMessage, PortFuture, RegisteredAgent, SecretFactory,
        StoredAgentInstanceRegistration,
    },
};

const MAX_SLUG_LENGTH: usize = 63;
const MAX_DISPLAY_NAME_LENGTH: usize = 128;
const MAX_DESCRIPTION_LENGTH: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAgent {
    pub request_id: AgentCreationRequestId,
    pub actor: AuthenticatedPrincipal,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub avatar_content_id: Option<ContentId>,
    pub visibility: AgentVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentCreationDraft {
    request_id: AgentCreationRequestId,
    owner_id: PrincipalId,
    slug: String,
    display_name: String,
    description: String,
    avatar_content_id: Option<ContentId>,
    visibility: AgentVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAgents {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureDefaultAgent {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureDefaultAgentForDevice {
    pub actor: AuthenticatedDevice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterAgentInstance {
    pub request_id: AgentInstanceRegistrationRequestId,
    pub actor: AuthenticatedDevice,
    pub agent_id: AgentId,
    pub adapter_type: String,
    pub external_subject_hash: Option<AdapterSubjectHash>,
    pub capability_version: String,
    pub configuration: Map<String, Value>,
    pub public_signing_key: AgentInstancePublicSigningKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAgentInstance {
    pub agent: RegisteredAgent,
    pub registration: StoredAgentInstanceRegistration,
    pub matrix_session: MatrixSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeAgentMembership {
    pub actor: AuthenticatedPrincipal,
    pub agent_id: AgentId,
    pub principal_id: PrincipalId,
    pub role: Option<AgentRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentManagementFailureKind {
    InvalidRequest,
    Forbidden,
    NotFound,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentManagementFailure {
    operation: &'static str,
    kind: AgentManagementFailureKind,
}

impl AgentManagementFailure {
    const fn new(operation: &'static str, kind: AgentManagementFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentManagementFailureKind {
        self.kind
    }
}

pub type AgentManagementResult<T> = Result<T, AgentManagementFailure>;

pub trait AgentManagementUseCases: Send + Sync {
    fn list_agents(
        &self,
        request: ListAgents,
    ) -> PortFuture<'_, AgentManagementResult<Vec<RegisteredAgent>>>;

    fn ensure_default_agent(
        &self,
        request: EnsureDefaultAgent,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>>;

    fn ensure_default_agent_for_device(
        &self,
        request: EnsureDefaultAgentForDevice,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>>;

    fn create_agent(
        &self,
        request: CreateAgent,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>>;

    fn register_instance(
        &self,
        request: RegisterAgentInstance,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgentInstance>>;

    fn change_membership(
        &self,
        request: ChangeAgentMembership,
    ) -> PortFuture<'_, AgentManagementResult<()>>;
}

pub struct AgentManagementService {
    creations: Arc<dyn AgentCreationWorkflow>,
    agents: Arc<dyn AgentRepository>,
    memberships: Arc<dyn AgentMembershipRepository>,
    membership_changes: Arc<dyn AgentMembershipTransaction>,
    instances: Arc<dyn AgentInstanceRegistrationTransaction>,
    matrix_identities: Arc<dyn MatrixAgentIdentityProvisioner>,
    secrets: Arc<dyn SecretFactory>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
}

pub struct AgentManagementDependencies {
    pub creations: Arc<dyn AgentCreationWorkflow>,
    pub agents: Arc<dyn AgentRepository>,
    pub memberships: Arc<dyn AgentMembershipRepository>,
    pub membership_changes: Arc<dyn AgentMembershipTransaction>,
    pub instances: Arc<dyn AgentInstanceRegistrationTransaction>,
    pub matrix_identities: Arc<dyn MatrixAgentIdentityProvisioner>,
    pub secrets: Arc<dyn SecretFactory>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl AgentManagementService {
    pub fn new(dependencies: AgentManagementDependencies) -> Self {
        Self {
            creations: dependencies.creations,
            agents: dependencies.agents,
            memberships: dependencies.memberships,
            membership_changes: dependencies.membership_changes,
            instances: dependencies.instances,
            matrix_identities: dependencies.matrix_identities,
            secrets: dependencies.secrets,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
        }
    }

    async fn create_agent_internal(
        &self,
        request: CreateAgent,
    ) -> AgentManagementResult<RegisteredAgent> {
        let operation = "agent.create";
        validate_agent_profile(&request)?;
        ensure_active_principal(&request.actor, self.clock.now(), operation)?;
        let proposed_agent_id = self.identifiers.agent_id();
        self.create_agent_with_id(AgentCreationDraft::from(request), proposed_agent_id)
            .await
    }

    async fn create_agent_with_id(
        &self,
        request: AgentCreationDraft,
        proposed_agent_id: AgentId,
    ) -> AgentManagementResult<RegisteredAgent> {
        let operation = "agent.create";
        let started_at = self.clock.now();
        let fingerprint = self.secrets.digest(&canonical_agent_creation(&request));
        let claim = AgentCreationClaim {
            request_id: request.request_id,
            owner_id: request.owner_id,
            proposed_agent_id,
            request_fingerprint: fingerprint,
            reserved_at: started_at,
        };
        let agent_id = match self
            .creations
            .reserve(&claim)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?
        {
            AgentCreationReservation::Completed(agent) => return Ok(agent),
            AgentCreationReservation::Reserved { agent_id } => agent_id,
        };

        let localpart = MatrixAgentLocalpart::from_agent_id(agent_id);
        let matrix_user_id = self
            .matrix_identities
            .ensure_user(&MatrixAgentUserRegistration::new(localpart.clone()))
            .await
            .map_err(|error| map_matrix_failure(operation, error.kind()))?;
        ensure_agent_matrix_identity(&localpart, &matrix_user_id, operation)?;
        let now = self.clock.now();
        let registration = AgentRegistration {
            agent: Agent::register(agent_id),
            owner_id: request.owner_id,
            matrix_user_id: matrix_user_id.as_str().to_owned(),
            slug: request.slug,
            display_name: request.display_name,
            description: request.description,
            avatar_content_id: request.avatar_content_id,
            visibility: request.visibility,
            registered_at: now,
        };
        let event = agent_registered_event(&*self.identifiers, &registration, now)?;
        self.creations
            .complete_with_event(request.request_id, &fingerprint, &registration, &event)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        Ok(RegisteredAgent::from(&registration))
    }

    async fn list_agents_internal(
        &self,
        request: ListAgents,
    ) -> AgentManagementResult<Vec<RegisteredAgent>> {
        let operation = "agent.list";
        ensure_active_principal(&request.actor, self.clock.now(), operation)?;
        self.agents
            .list_for_principal(request.actor.principal_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))
    }

    async fn ensure_default_agent_internal(
        &self,
        request: EnsureDefaultAgent,
    ) -> AgentManagementResult<RegisteredAgent> {
        let operation = "agent.ensure_default";
        ensure_active_principal(&request.actor, self.clock.now(), operation)?;
        self.ensure_default_agent_for_owner(request.actor.principal_id, operation)
            .await
    }

    async fn ensure_default_agent_for_device_internal(
        &self,
        request: EnsureDefaultAgentForDevice,
    ) -> AgentManagementResult<RegisteredAgent> {
        let operation = "agent.ensure_default_for_device";
        ensure_active_device(&request.actor, self.clock.now(), operation)?;
        self.ensure_default_agent_for_owner(request.actor.account.principal.id(), operation)
            .await
    }

    async fn ensure_default_agent_for_owner(
        &self,
        principal_id: PrincipalId,
        operation: &'static str,
    ) -> AgentManagementResult<RegisteredAgent> {
        let existing = self
            .agents
            .list_for_principal(principal_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        if let Some(agent) = existing.into_iter().next() {
            return Ok(agent);
        }

        let principal_uuid = principal_id.as_uuid();
        let compact = principal_uuid.simple().to_string();
        self.create_agent_with_id(
            AgentCreationDraft {
                request_id: AgentCreationRequestId::from_uuid(principal_uuid),
                owner_id: principal_id,
                slug: format!("agent-{}", &compact[compact.len() - 12..]),
                display_name: format!("Agent {}", &compact[..8]),
                description: String::new(),
                avatar_content_id: None,
                visibility: AgentVisibility::Private,
            },
            AgentId::from_uuid(principal_uuid),
        )
        .await
    }

    async fn register_instance_internal(
        &self,
        request: RegisterAgentInstance,
    ) -> AgentManagementResult<RegisteredAgentInstance> {
        let operation = "agent_instance.register";
        ensure_active_device(&request.actor, self.clock.now(), operation)?;
        let memberships = self
            .memberships
            .find_memberships(request.agent_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, AgentManagementFailureKind::NotFound))?;
        memberships
            .ensure_can_register_instance(request.actor.account.principal.id())
            .map_err(|error| map_domain_failure(operation, &error))?;
        let agent = self
            .agents
            .find_registration(request.agent_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, AgentManagementFailureKind::NotFound))?;

        let fingerprint = self
            .secrets
            .digest(&canonical_instance_registration(&request)?);
        let binding_id = self.identifiers.adapter_binding_id();
        let binding = AdapterBinding::register(
            binding_id,
            request.agent_id,
            request.adapter_type.clone(),
            request.external_subject_hash.clone(),
            request.capability_version.clone(),
        )
        .map_err(|error| map_domain_failure(operation, &error))?;
        let instance_id = self.identifiers.agent_instance_id();
        let matrix_device_id =
            AgentMatrixDeviceId::new(format!("AR_{}", instance_id.as_uuid().simple()))
                .map_err(|error| map_domain_failure(operation, &error))?;
        let instance = AgentInstance::register(
            instance_id,
            request.agent_id,
            request.actor.device_id,
            binding_id,
            request.public_signing_key,
            matrix_device_id,
        );
        let now = self.clock.now();
        let registration = AgentInstanceRegistration {
            request_id: request.request_id,
            principal_id: request.actor.account.principal.id(),
            device_id: request.actor.device_id,
            request_fingerprint: fingerprint,
            binding,
            binding_configuration: request.configuration,
            instance,
            registered_at: now,
        };
        let event = agent_instance_registered_event(&*self.identifiers, &registration, now)?;
        let stored = self
            .instances
            .register_with_event(&registration, &event)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;

        let matrix_user_id = MatrixUserId::new(agent.matrix_user_id.clone())
            .map_err(|_| internal_failure(operation))?;
        let matrix_device_id =
            MatrixDeviceId::new(stored.instance.matrix_device_id().as_str().to_owned())
                .map_err(|_| internal_failure(operation))?;
        let session_request = MatrixAgentDeviceSessionRequest::new(
            matrix_user_id.clone(),
            matrix_device_id.clone(),
            format!("Agent Room · {}", stored.binding.adapter_type()),
        )
        .map_err(|_| internal_failure(operation))?;
        let session = self
            .matrix_identities
            .issue_device_session(&session_request)
            .await
            .map_err(|error| map_matrix_failure(operation, error.kind()))?;
        if session.metadata().user_id() != &matrix_user_id
            || session.metadata().device_id() != &matrix_device_id
        {
            return Err(internal_failure(operation));
        }
        Ok(RegisteredAgentInstance {
            agent,
            registration: stored,
            matrix_session: session,
        })
    }

    async fn change_membership_internal(
        &self,
        request: ChangeAgentMembership,
    ) -> AgentManagementResult<()> {
        let operation = "agent_membership.change";
        let now = self.clock.now();
        ensure_active_principal(&request.actor, now, operation)?;
        let change = AgentMembershipChange {
            agent_id: request.agent_id,
            actor_id: request.actor.principal_id,
            principal_id: request.principal_id,
            role: request.role,
            changed_at: now,
        };
        let event = agent_membership_changed_event(&*self.identifiers, &change, now)?;
        self.membership_changes
            .apply_change(&change, &event)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        Ok(())
    }
}

impl AgentManagementUseCases for AgentManagementService {
    fn list_agents(
        &self,
        request: ListAgents,
    ) -> PortFuture<'_, AgentManagementResult<Vec<RegisteredAgent>>> {
        Box::pin(self.list_agents_internal(request))
    }

    fn ensure_default_agent(
        &self,
        request: EnsureDefaultAgent,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        Box::pin(self.ensure_default_agent_internal(request))
    }

    fn ensure_default_agent_for_device(
        &self,
        request: EnsureDefaultAgentForDevice,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        Box::pin(self.ensure_default_agent_for_device_internal(request))
    }

    fn create_agent(
        &self,
        request: CreateAgent,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        Box::pin(self.create_agent_internal(request))
    }

    fn register_instance(
        &self,
        request: RegisterAgentInstance,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgentInstance>> {
        Box::pin(self.register_instance_internal(request))
    }

    fn change_membership(
        &self,
        request: ChangeAgentMembership,
    ) -> PortFuture<'_, AgentManagementResult<()>> {
        Box::pin(self.change_membership_internal(request))
    }
}

fn validate_agent_profile(request: &CreateAgent) -> AgentManagementResult<()> {
    let valid_slug = !request.slug.is_empty()
        && request.slug.len() <= MAX_SLUG_LENGTH
        && request.slug.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (byte == b'-' && index > 0)
        });
    let valid_name = valid_text(&request.display_name, MAX_DISPLAY_NAME_LENGTH, false);
    let valid_description = valid_text(&request.description, MAX_DESCRIPTION_LENGTH, true);
    if valid_slug && valid_name && valid_description {
        Ok(())
    } else {
        Err(failure(
            "agent.create",
            AgentManagementFailureKind::InvalidRequest,
        ))
    }
}

impl From<CreateAgent> for AgentCreationDraft {
    fn from(value: CreateAgent) -> Self {
        Self {
            request_id: value.request_id,
            owner_id: value.actor.principal_id,
            slug: value.slug,
            display_name: value.display_name,
            description: value.description,
            avatar_content_id: value.avatar_content_id,
            visibility: value.visibility,
        }
    }
}

fn valid_text(value: &str, maximum: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}

fn ensure_active_principal(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> AgentManagementResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(operation, AgentManagementFailureKind::Forbidden))
    }
}

fn ensure_active_device(
    actor: &AuthenticatedDevice,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> AgentManagementResult<()> {
    if actor.account.principal.allows_authentication() && now < actor.access_token_expires_at {
        Ok(())
    } else {
        Err(failure(operation, AgentManagementFailureKind::Forbidden))
    }
}

fn ensure_agent_matrix_identity(
    localpart: &MatrixAgentLocalpart,
    user_id: &MatrixUserId,
    operation: &'static str,
) -> AgentManagementResult<()> {
    let expected = format!("@{}:", localpart.as_str());
    if user_id.as_str().starts_with(&expected) {
        Ok(())
    } else {
        Err(internal_failure(operation))
    }
}

fn canonical_agent_creation(request: &AgentCreationDraft) -> String {
    let mut canonical = CanonicalRequest::new("agent.create.v1");
    canonical.field("principal_id", request.owner_id.to_string());
    canonical.field("slug", &request.slug);
    canonical.field("display_name", &request.display_name);
    canonical.field("description", &request.description);
    canonical.field(
        "avatar_content_id",
        request
            .avatar_content_id
            .map_or_else(String::new, |id| id.to_string()),
    );
    canonical.field("visibility", request.visibility.as_str());
    canonical.finish()
}

fn canonical_instance_registration(
    request: &RegisterAgentInstance,
) -> AgentManagementResult<String> {
    let mut canonical = CanonicalRequest::new("agent.instance.register.v1");
    canonical.field(
        "principal_id",
        request.actor.account.principal.id().to_string(),
    );
    canonical.field("device_id", request.actor.device_id.to_string());
    canonical.field("agent_id", request.agent_id.to_string());
    canonical.field("adapter_type", &request.adapter_type);
    canonical.field(
        "external_subject_hash",
        request
            .external_subject_hash
            .as_ref()
            .map_or_else(String::new, |hash| encode_hex(hash.as_bytes())),
    );
    canonical.field("capability_version", &request.capability_version);
    canonical.field(
        "configuration",
        serde_json::to_string(&request.configuration)
            .map_err(|_| internal_failure("agent_instance.register"))?,
    );
    canonical.field(
        "public_signing_key",
        encode_hex(request.public_signing_key.as_bytes()),
    );
    Ok(canonical.finish())
}

struct CanonicalRequest(String);

impl CanonicalRequest {
    fn new(schema: &str) -> Self {
        let mut value = Self(String::new());
        value.field("schema", schema);
        value
    }

    fn field(&mut self, name: &str, value: impl AsRef<str>) {
        let value = value.as_ref();
        let _ = writeln!(self.0, "{}:{}:{}", name.len(), name, value.len());
        self.0.push_str(value);
        self.0.push('\n');
    }

    fn finish(self) -> String {
        self.0
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn agent_registered_event(
    identifiers: &dyn IdentifierFactory,
    registration: &AgentRegistration,
    occurred_at: agent_room_domain::time::UtcMillis,
) -> AgentManagementResult<OutboxMessage> {
    let mut payload = Map::new();
    payload.insert(
        "owner_id".to_owned(),
        Value::String(registration.owner_id.to_string()),
    );
    payload.insert(
        "matrix_user_id".to_owned(),
        Value::String(registration.matrix_user_id.clone()),
    );
    OutboxMessage::new(
        identifiers.outbox_event_id(),
        "agent".to_owned(),
        registration.agent.id().as_uuid(),
        "agent.registered.v1".to_owned(),
        payload,
        occurred_at,
    )
    .map_err(|_| internal_failure("agent.create"))
}

fn agent_instance_registered_event(
    identifiers: &dyn IdentifierFactory,
    registration: &AgentInstanceRegistration,
    occurred_at: agent_room_domain::time::UtcMillis,
) -> AgentManagementResult<OutboxMessage> {
    let mut payload = Map::new();
    payload.insert(
        "agent_id".to_owned(),
        Value::String(registration.instance.agent_id().to_string()),
    );
    payload.insert(
        "device_id".to_owned(),
        Value::String(registration.device_id.to_string()),
    );
    OutboxMessage::new(
        identifiers.outbox_event_id(),
        "agent_instance".to_owned(),
        registration.instance.id().as_uuid(),
        "agent.instance.registered.v1".to_owned(),
        payload,
        occurred_at,
    )
    .map_err(|_| internal_failure("agent_instance.register"))
}

fn agent_membership_changed_event(
    identifiers: &dyn IdentifierFactory,
    change: &AgentMembershipChange,
    occurred_at: agent_room_domain::time::UtcMillis,
) -> AgentManagementResult<OutboxMessage> {
    let mut payload = Map::new();
    payload.insert(
        "principal_id".to_owned(),
        Value::String(change.principal_id.to_string()),
    );
    payload.insert(
        "role".to_owned(),
        change
            .role
            .map_or(Value::Null, |role| Value::String(role.as_str().to_owned())),
    );
    OutboxMessage::new(
        identifiers.outbox_event_id(),
        "agent".to_owned(),
        change.agent_id.as_uuid(),
        "agent.membership.changed.v1".to_owned(),
        payload,
        occurred_at,
    )
    .map_err(|_| internal_failure("agent_membership.change"))
}

const fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> AgentManagementFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => AgentManagementFailureKind::Conflict,
        RepositoryErrorKind::Constraint => AgentManagementFailureKind::InvalidRequest,
        RepositoryErrorKind::Forbidden => AgentManagementFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => AgentManagementFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => AgentManagementFailureKind::DependencyUnavailable,
        RepositoryErrorKind::CorruptData => AgentManagementFailureKind::Internal,
    };
    AgentManagementFailure::new(operation, kind)
}

const fn map_matrix_failure(
    operation: &'static str,
    kind: MatrixFailureKind,
) -> AgentManagementFailure {
    let kind = match kind {
        MatrixFailureKind::Conflict => AgentManagementFailureKind::Conflict,
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => AgentManagementFailureKind::DependencyUnavailable,
        MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => AgentManagementFailureKind::Internal,
    };
    AgentManagementFailure::new(operation, kind)
}

const fn map_domain_failure(
    operation: &'static str,
    error: &DomainError,
) -> AgentManagementFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => AgentManagementFailureKind::Forbidden,
        DomainError::Validation { .. }
        | DomainError::InvariantViolation { .. }
        | DomainError::InvalidTransition { .. }
        | DomainError::CapacityExceeded { .. } => AgentManagementFailureKind::InvalidRequest,
        DomainError::TimeOverflow | DomainError::VersionOverflow => {
            AgentManagementFailureKind::Internal
        }
    };
    AgentManagementFailure::new(operation, kind)
}

const fn failure(
    operation: &'static str,
    kind: AgentManagementFailureKind,
) -> AgentManagementFailure {
    AgentManagementFailure::new(operation, kind)
}

const fn internal_failure(operation: &'static str) -> AgentManagementFailure {
    failure(operation, AgentManagementFailureKind::Internal)
}
