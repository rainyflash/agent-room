use std::sync::Arc;

use agent_room_domain::ids::AgentInstanceId;

use crate::{
    devices::AuthenticatedDevice,
    persistence::RepositoryErrorKind,
    ports::{
        AgentInstanceVerificationRecord, AgentInstanceVerificationRepository, Clock, PortFuture,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveAgentInstanceVerification {
    pub actor: AuthenticatedDevice,
    pub instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstanceVerificationFailureKind {
    Unauthorized,
    NotFound,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentInstanceVerificationFailure {
    kind: AgentInstanceVerificationFailureKind,
}

impl AgentInstanceVerificationFailure {
    const fn new(kind: AgentInstanceVerificationFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> AgentInstanceVerificationFailureKind {
        self.kind
    }
}

pub type AgentInstanceVerificationResult<T> = Result<T, AgentInstanceVerificationFailure>;

pub trait AgentInstanceVerificationUseCases: Send + Sync {
    fn resolve(
        &self,
        request: ResolveAgentInstanceVerification,
    ) -> PortFuture<'_, AgentInstanceVerificationResult<AgentInstanceVerificationRecord>>;
}

pub struct AgentInstanceVerificationService {
    records: Arc<dyn AgentInstanceVerificationRepository>,
    clock: Arc<dyn Clock>,
}

pub struct AgentInstanceVerificationDependencies {
    pub records: Arc<dyn AgentInstanceVerificationRepository>,
    pub clock: Arc<dyn Clock>,
}

impl AgentInstanceVerificationService {
    pub fn new(dependencies: AgentInstanceVerificationDependencies) -> Self {
        Self {
            records: dependencies.records,
            clock: dependencies.clock,
        }
    }

    async fn resolve_internal(
        &self,
        request: ResolveAgentInstanceVerification,
    ) -> AgentInstanceVerificationResult<AgentInstanceVerificationRecord> {
        if request.actor.access_token_expires_at <= self.clock.now() {
            return Err(failure(AgentInstanceVerificationFailureKind::Unauthorized));
        }
        let record = self
            .records
            .find_verification_record(request.instance_id)
            .await
            .map_err(|error| match error.kind() {
                RepositoryErrorKind::Unavailable => {
                    failure(AgentInstanceVerificationFailureKind::DependencyUnavailable)
                }
                RepositoryErrorKind::Forbidden => {
                    failure(AgentInstanceVerificationFailureKind::Unauthorized)
                }
                RepositoryErrorKind::NotFound => {
                    failure(AgentInstanceVerificationFailureKind::NotFound)
                }
                RepositoryErrorKind::Conflict
                | RepositoryErrorKind::Constraint
                | RepositoryErrorKind::CorruptData => {
                    failure(AgentInstanceVerificationFailureKind::Internal)
                }
            })?
            .ok_or_else(|| failure(AgentInstanceVerificationFailureKind::NotFound))?;
        if record.instance_id != request.instance_id
            || record
                .invalidated_at
                .is_some_and(|invalidated_at| invalidated_at < record.registered_at)
        {
            return Err(failure(AgentInstanceVerificationFailureKind::Internal));
        }
        Ok(record)
    }
}

impl AgentInstanceVerificationUseCases for AgentInstanceVerificationService {
    fn resolve(
        &self,
        request: ResolveAgentInstanceVerification,
    ) -> PortFuture<'_, AgentInstanceVerificationResult<AgentInstanceVerificationRecord>> {
        Box::pin(self.resolve_internal(request))
    }
}

const fn failure(kind: AgentInstanceVerificationFailureKind) -> AgentInstanceVerificationFailure {
    AgentInstanceVerificationFailure::new(kind)
}
