use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentInstanceId, PrincipalId},
    time::UtcMillis,
};
use serde_json::{Map, Value};

use crate::{
    authentication::AuthenticatedPrincipal,
    matrix_device_cleanup::{MatrixDeviceCleanupFailure, revoke_agent_matrix_device},
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentInstanceManagementRecord, AgentInstanceManagementRepository,
        AgentInstanceMatrixCleanupStore, AgentInstanceRevocationOutcome,
        AgentInstanceRevocationTransaction, Clock, IdentifierFactory,
        MatrixAgentDeviceSessionRevoker, MatrixFailureKind, OutboxMessage, PortFuture,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAgentInstances {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeAgentInstance {
    pub actor: AuthenticatedPrincipal,
    pub instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceCleanupFailureKind {
    DependencyUnavailable,
    Rejected,
    Unsupported,
    InvalidStoredIdentity,
    StatePersistenceUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceMatrixCleanup {
    Complete,
    Pending {
        reason: AgentInstanceCleanupFailureKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokedAgentInstance {
    pub instance: AgentInstanceManagementRecord,
    pub matrix_cleanup: AgentInstanceMatrixCleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceManagementFailureKind {
    Forbidden,
    NotFound,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentInstanceManagementFailure {
    operation: &'static str,
    kind: AgentInstanceManagementFailureKind,
}

impl AgentInstanceManagementFailure {
    const fn new(operation: &'static str, kind: AgentInstanceManagementFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentInstanceManagementFailureKind {
        self.kind
    }
}

pub type AgentInstanceManagementResult<T> = Result<T, AgentInstanceManagementFailure>;

pub trait AgentInstanceManagementUseCases: Send + Sync {
    fn list_instances(
        &self,
        request: ListAgentInstances,
    ) -> PortFuture<'_, AgentInstanceManagementResult<Vec<AgentInstanceManagementRecord>>>;

    fn revoke_instance(
        &self,
        request: RevokeAgentInstance,
    ) -> PortFuture<'_, AgentInstanceManagementResult<RevokedAgentInstance>>;
}

pub struct AgentInstanceManagementService {
    instances: Arc<dyn AgentInstanceManagementRepository>,
    revocations: Arc<dyn AgentInstanceRevocationTransaction>,
    matrix_cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
}

pub struct AgentInstanceManagementDependencies {
    pub instances: Arc<dyn AgentInstanceManagementRepository>,
    pub revocations: Arc<dyn AgentInstanceRevocationTransaction>,
    pub matrix_cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    pub matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl AgentInstanceManagementService {
    pub fn new(dependencies: AgentInstanceManagementDependencies) -> Self {
        Self {
            instances: dependencies.instances,
            revocations: dependencies.revocations,
            matrix_cleanup: dependencies.matrix_cleanup,
            matrix: dependencies.matrix,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
        }
    }

    async fn list_instances_internal(
        &self,
        request: ListAgentInstances,
    ) -> AgentInstanceManagementResult<Vec<AgentInstanceManagementRecord>> {
        let operation = "agent_instance.list";
        ensure_active_actor(&request.actor, self.clock.now(), operation)?;
        self.instances
            .list_for_principal(request.actor.principal_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))
    }

    async fn revoke_instance_internal(
        &self,
        request: RevokeAgentInstance,
    ) -> AgentInstanceManagementResult<RevokedAgentInstance> {
        let operation = "agent_instance.revoke";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, operation)?;
        let event = revocation_event(
            self.identifiers.as_ref(),
            request.actor.principal_id,
            request.instance_id,
            now,
        )?;
        let outcome = self
            .revocations
            .revoke(request.actor.principal_id, request.instance_id, &event)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        let instance = match outcome {
            AgentInstanceRevocationOutcome::Revoked(instance)
            | AgentInstanceRevocationOutcome::AlreadyRevoked(instance) => instance,
            AgentInstanceRevocationOutcome::NotFound => {
                return Err(failure(
                    operation,
                    AgentInstanceManagementFailureKind::NotFound,
                ));
            }
        };

        if instance.matrix_device_revoked_at.is_some() {
            return Ok(RevokedAgentInstance {
                instance,
                matrix_cleanup: AgentInstanceMatrixCleanup::Complete,
            });
        }

        let cleanup = revoke_agent_matrix_device(
            self.matrix.as_ref(),
            self.matrix_cleanup.as_ref(),
            request.instance_id,
            &instance.agent_matrix_user_id,
            instance.instance.matrix_device_id().as_str(),
            now,
        )
        .await;
        if let Err(cleanup_failure) = cleanup {
            return Ok(pending_cleanup(
                instance,
                map_matrix_cleanup_failure(cleanup_failure),
            ));
        }
        let mut instance = instance;
        instance.matrix_device_revoked_at = Some(now);
        Ok(RevokedAgentInstance {
            instance,
            matrix_cleanup: AgentInstanceMatrixCleanup::Complete,
        })
    }
}

impl AgentInstanceManagementUseCases for AgentInstanceManagementService {
    fn list_instances(
        &self,
        request: ListAgentInstances,
    ) -> PortFuture<'_, AgentInstanceManagementResult<Vec<AgentInstanceManagementRecord>>> {
        Box::pin(self.list_instances_internal(request))
    }

    fn revoke_instance(
        &self,
        request: RevokeAgentInstance,
    ) -> PortFuture<'_, AgentInstanceManagementResult<RevokedAgentInstance>> {
        Box::pin(self.revoke_instance_internal(request))
    }
}

const fn map_matrix_cleanup_failure(
    failure: MatrixDeviceCleanupFailure,
) -> AgentInstanceCleanupFailureKind {
    match failure {
        MatrixDeviceCleanupFailure::InvalidStoredIdentity => {
            AgentInstanceCleanupFailureKind::InvalidStoredIdentity
        }
        MatrixDeviceCleanupFailure::StatePersistenceUnavailable => {
            AgentInstanceCleanupFailureKind::StatePersistenceUnavailable
        }
        MatrixDeviceCleanupFailure::Matrix(kind) => map_matrix_failure_kind(kind),
    }
}

const fn map_matrix_failure_kind(kind: MatrixFailureKind) -> AgentInstanceCleanupFailureKind {
    match kind {
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => {
            AgentInstanceCleanupFailureKind::DependencyUnavailable
        }
        MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden => AgentInstanceCleanupFailureKind::Rejected,
        MatrixFailureKind::UnsupportedVersion => AgentInstanceCleanupFailureKind::Unsupported,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::Conflict
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken => {
            AgentInstanceCleanupFailureKind::InvalidStoredIdentity
        }
    }
}

const fn pending_cleanup(
    instance: AgentInstanceManagementRecord,
    reason: AgentInstanceCleanupFailureKind,
) -> RevokedAgentInstance {
    RevokedAgentInstance {
        instance,
        matrix_cleanup: AgentInstanceMatrixCleanup::Pending { reason },
    }
}

fn revocation_event(
    identifiers: &dyn IdentifierFactory,
    principal_id: PrincipalId,
    instance_id: AgentInstanceId,
    occurred_at: UtcMillis,
) -> AgentInstanceManagementResult<OutboxMessage> {
    let mut payload = Map::new();
    payload.insert(
        "principal_id".to_owned(),
        Value::String(principal_id.to_string()),
    );
    OutboxMessage::new(
        identifiers.outbox_event_id(),
        "agent_instance".to_owned(),
        instance_id.as_uuid(),
        "agent.instance.revoked.v1".to_owned(),
        payload,
        occurred_at,
    )
    .map_err(|_| {
        failure(
            "agent_instance.revoke",
            AgentInstanceManagementFailureKind::Internal,
        )
    })
}

fn ensure_active_actor(
    actor: &AuthenticatedPrincipal,
    now: UtcMillis,
    operation: &'static str,
) -> AgentInstanceManagementResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(
            operation,
            AgentInstanceManagementFailureKind::Forbidden,
        ))
    }
}

const fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> AgentInstanceManagementFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Forbidden => AgentInstanceManagementFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => AgentInstanceManagementFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => {
            AgentInstanceManagementFailureKind::DependencyUnavailable
        }
        RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => AgentInstanceManagementFailureKind::Internal,
    };
    failure(operation, kind)
}

const fn failure(
    operation: &'static str,
    kind: AgentInstanceManagementFailureKind,
) -> AgentInstanceManagementFailure {
    AgentInstanceManagementFailure::new(operation, kind)
}
