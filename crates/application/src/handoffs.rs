use std::sync::Arc;

use agent_room_domain::{
    agents::AgentRole,
    ids::{AgentId, AgentInstanceId, PrincipalId},
};

use crate::{
    devices::AuthenticatedDevice,
    persistence::RepositoryErrorKind,
    ports::{Clock, HandoffAccessRepository, HandoffInstanceAccessRecord, PortFuture},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeHandoff {
    pub actor: AuthenticatedDevice,
    pub principal_id: PrincipalId,
    pub requester_agent_id: AgentId,
    pub requester_instance_id: AgentInstanceId,
    pub target_agent_id: AgentId,
    pub target_instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveHandoffInstance {
    pub actor: AuthenticatedDevice,
    pub instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffAuthorizationDecision {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHandoffInstance {
    pub agent_id: AgentId,
    pub instance_id: AgentInstanceId,
    pub matrix_user_id: String,
    pub matrix_device_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffAccessFailureKind {
    Unauthorized,
    NotFound,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoffAccessFailure {
    operation: &'static str,
    kind: HandoffAccessFailureKind,
}

impl HandoffAccessFailure {
    const fn new(operation: &'static str, kind: HandoffAccessFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> HandoffAccessFailureKind {
        self.kind
    }
}

pub type HandoffAccessResult<T> = Result<T, HandoffAccessFailure>;

pub trait HandoffAccessUseCases: Send + Sync {
    fn authorize(
        &self,
        request: AuthorizeHandoff,
    ) -> PortFuture<'_, HandoffAccessResult<HandoffAuthorizationDecision>>;

    fn resolve_instance(
        &self,
        request: ResolveHandoffInstance,
    ) -> PortFuture<'_, HandoffAccessResult<ResolvedHandoffInstance>>;
}

pub struct HandoffAccessService {
    access: Arc<dyn HandoffAccessRepository>,
    clock: Arc<dyn Clock>,
}

pub struct HandoffAccessDependencies {
    pub access: Arc<dyn HandoffAccessRepository>,
    pub clock: Arc<dyn Clock>,
}

impl HandoffAccessService {
    pub fn new(dependencies: HandoffAccessDependencies) -> Self {
        Self {
            access: dependencies.access,
            clock: dependencies.clock,
        }
    }

    async fn authorize_internal(
        &self,
        request: AuthorizeHandoff,
    ) -> HandoffAccessResult<HandoffAuthorizationDecision> {
        const OPERATION: &str = "handoff.authorize";
        self.ensure_actor(&request.actor, OPERATION)?;
        if request.actor.account.principal.id() != request.principal_id {
            return Ok(HandoffAuthorizationDecision::Denied);
        }
        let snapshot = self
            .access
            .inspect_authorization(
                request.principal_id,
                request.requester_instance_id,
                request.target_instance_id,
            )
            .await
            .map_err(|error| map_repository_failure(OPERATION, error.kind()))?;
        let Some(snapshot) = snapshot else {
            return Ok(HandoffAuthorizationDecision::Denied);
        };
        if snapshot.requester.instance_id != request.requester_instance_id
            || snapshot.requester.agent_id != request.requester_agent_id
            || snapshot.target.instance_id != request.target_instance_id
            || snapshot.target.agent_id != request.target_agent_id
        {
            return Ok(HandoffAuthorizationDecision::Denied);
        }
        if can_receive_handoff(&snapshot.requester) && can_receive_handoff(&snapshot.target) {
            Ok(HandoffAuthorizationDecision::Allowed)
        } else {
            Ok(HandoffAuthorizationDecision::Denied)
        }
    }

    async fn resolve_instance_internal(
        &self,
        request: ResolveHandoffInstance,
    ) -> HandoffAccessResult<ResolvedHandoffInstance> {
        const OPERATION: &str = "handoff.instance.resolve";
        self.ensure_actor(&request.actor, OPERATION)?;
        let principal_id = request.actor.account.principal.id();
        let record = self
            .access
            .find_instance_access(principal_id, request.instance_id)
            .await
            .map_err(|error| map_repository_failure(OPERATION, error.kind()))?
            .ok_or_else(|| failure(OPERATION, HandoffAccessFailureKind::NotFound))?;
        if record.instance_id != request.instance_id {
            return Err(failure(OPERATION, HandoffAccessFailureKind::Internal));
        }
        if !can_receive_handoff(&record) {
            return Err(failure(OPERATION, HandoffAccessFailureKind::NotFound));
        }
        if !valid_matrix_identity(&record.matrix_user_id, &record.matrix_device_id) {
            return Err(failure(OPERATION, HandoffAccessFailureKind::Internal));
        }
        Ok(ResolvedHandoffInstance {
            agent_id: record.agent_id,
            instance_id: record.instance_id,
            matrix_user_id: record.matrix_user_id,
            matrix_device_id: record.matrix_device_id,
        })
    }

    fn ensure_actor(
        &self,
        actor: &AuthenticatedDevice,
        operation: &'static str,
    ) -> HandoffAccessResult<()> {
        if actor.account.principal.allows_authentication()
            && self.clock.now() < actor.access_token_expires_at
        {
            Ok(())
        } else {
            Err(failure(operation, HandoffAccessFailureKind::Unauthorized))
        }
    }
}

impl HandoffAccessUseCases for HandoffAccessService {
    fn authorize(
        &self,
        request: AuthorizeHandoff,
    ) -> PortFuture<'_, HandoffAccessResult<HandoffAuthorizationDecision>> {
        Box::pin(self.authorize_internal(request))
    }

    fn resolve_instance(
        &self,
        request: ResolveHandoffInstance,
    ) -> PortFuture<'_, HandoffAccessResult<ResolvedHandoffInstance>> {
        Box::pin(self.resolve_instance_internal(request))
    }
}

fn can_receive_handoff(record: &HandoffInstanceAccessRecord) -> bool {
    record.active && record.role.is_some_and(AgentRole::can_register_instance)
}

fn valid_matrix_identity(user_id: &str, device_id: &str) -> bool {
    user_id.starts_with('@')
        && user_id.contains(':')
        && (4..=512).contains(&user_id.len())
        && !user_id.chars().any(char::is_control)
        && (1..=255).contains(&device_id.len())
        && !device_id.chars().any(char::is_control)
}

const fn map_repository_failure(
    operation: &'static str,
    kind: RepositoryErrorKind,
) -> HandoffAccessFailure {
    let failure_kind = match kind {
        RepositoryErrorKind::Unavailable => HandoffAccessFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden => HandoffAccessFailureKind::Unauthorized,
        RepositoryErrorKind::NotFound => HandoffAccessFailureKind::NotFound,
        RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => HandoffAccessFailureKind::Internal,
    };
    failure(operation, failure_kind)
}

const fn failure(operation: &'static str, kind: HandoffAccessFailureKind) -> HandoffAccessFailure {
    HandoffAccessFailure::new(operation, kind)
}
