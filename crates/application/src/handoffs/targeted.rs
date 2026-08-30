use std::sync::Arc;

use agent_room_domain::{
    agents::AgentInstanceStatus,
    content::ContentLifecycleState,
    handoff::{
        HandoffContentReference, HandoffPermissions, HandoffPurpose, HandoffSourceEventId,
        TargetedHandoff, TargetedHandoffFields,
    },
    ids::{AgentInstanceId, ContentId, HandoffId, MessageId},
    rooms::MatrixRoomReference,
    time::{DurationMillis, UtcMillis},
};
use sha2::{Digest, Sha256};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        ClaimTargetedHandoff, Clock, ContentAccessMode, ContentAuthorizationDecision,
        ContentAuthorizationFailureKind, ContentAuthorizationIntent, ContentAuthorizationRequest,
        ContentMembershipAuthorizer, ContentRepository, MatrixRoomId, PortFuture,
        QueueTargetedHandoff, QueueTargetedHandoffOutcome, RecordTargetedHandoffReceipt,
        TargetedHandoffReceiptOutcome, TargetedHandoffRepository,
        TargetedHandoffRequestFingerprint, TargetedHandoffTargetRecord,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetedHandoffPolicy {
    minimum_ttl: DurationMillis,
    maximum_ttl: DurationMillis,
}

impl TargetedHandoffPolicy {
    /// 创建交接授权窗口策略。
    ///
    /// # Errors
    ///
    /// 最短期限大于最长期限时返回配置错误。
    pub const fn new(
        minimum_ttl: DurationMillis,
        maximum_ttl: DurationMillis,
    ) -> Result<Self, TargetedHandoffConfigurationError> {
        if minimum_ttl.value() > maximum_ttl.value() {
            return Err(TargetedHandoffConfigurationError::InvalidLifetimeRange);
        }
        Ok(Self {
            minimum_ttl,
            maximum_ttl,
        })
    }

    fn validate(self, now: UtcMillis, expires_at: UtcMillis) -> TargetedHandoffResult<()> {
        let minimum = now
            .checked_add(self.minimum_ttl)
            .map_err(|_| failure("handoff.create", TargetedHandoffFailureKind::InvalidRequest))?;
        let maximum = now
            .checked_add(self.maximum_ttl)
            .map_err(|_| failure("handoff.create", TargetedHandoffFailureKind::InvalidRequest))?;
        if expires_at < minimum || expires_at > maximum {
            return Err(failure(
                "handoff.create",
                TargetedHandoffFailureKind::InvalidRequest,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetedHandoffConfigurationError {
    InvalidLifetimeRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListTargetedHandoffTargets {
    pub actor: AuthenticatedPrincipal,
    pub room_id: MatrixRoomReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTargetedHandoff {
    pub handoff_id: HandoffId,
    pub actor: AuthenticatedPrincipal,
    pub source_room_id: MatrixRoomReference,
    pub source_event_id: HandoffSourceEventId,
    pub source_message_id: MessageId,
    pub target_instance_id: AgentInstanceId,
    pub content_id: ContentId,
    pub permissions: HandoffPermissions,
    pub purpose: HandoffPurpose,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetTargetedHandoff {
    pub actor: AuthenticatedPrincipal,
    pub handoff_id: HandoffId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeTargetedHandoff {
    pub actor: AuthenticatedPrincipal,
    pub handoff_id: HandoffId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimNextTargetedHandoff {
    pub actor: AuthenticatedDevice,
    pub target_instance_id: AgentInstanceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTargetedHandoffReceiptCommand {
    pub actor: AuthenticatedDevice,
    pub target_instance_id: AgentInstanceId,
    pub handoff_id: HandoffId,
    pub outcome: TargetedHandoffReceiptOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffTargetView {
    pub record: TargetedHandoffTargetRecord,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTargetedHandoffOutcome {
    pub handoff: TargetedHandoff,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetedHandoffFailureKind {
    Unauthorized,
    Forbidden,
    InvalidRequest,
    InvalidSource,
    TargetUnavailable,
    NotFound,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetedHandoffFailure {
    operation: &'static str,
    kind: TargetedHandoffFailureKind,
}

impl TargetedHandoffFailure {
    const fn new(operation: &'static str, kind: TargetedHandoffFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> TargetedHandoffFailureKind {
        self.kind
    }
}

pub type TargetedHandoffResult<T> = Result<T, TargetedHandoffFailure>;

pub trait TargetedHandoffUseCases: Send + Sync {
    fn list_targets(
        &self,
        request: ListTargetedHandoffTargets,
    ) -> PortFuture<'_, TargetedHandoffResult<Vec<HandoffTargetView>>>;

    fn create(
        &self,
        request: CreateTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<CreateTargetedHandoffOutcome>>;

    fn get(
        &self,
        request: GetTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<TargetedHandoff>>;

    fn revoke(
        &self,
        request: RevokeTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<TargetedHandoff>>;

    fn claim_next(
        &self,
        request: ClaimNextTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<Option<TargetedHandoff>>>;

    fn record_receipt(
        &self,
        request: RecordTargetedHandoffReceiptCommand,
    ) -> PortFuture<'_, TargetedHandoffResult<TargetedHandoff>>;
}

pub struct TargetedHandoffDependencies {
    pub store: Arc<dyn TargetedHandoffRepository>,
    pub content: Arc<dyn ContentRepository>,
    pub authorizer: Arc<dyn ContentMembershipAuthorizer>,
    pub clock: Arc<dyn Clock>,
    pub policy: TargetedHandoffPolicy,
}

pub struct TargetedHandoffService {
    store: Arc<dyn TargetedHandoffRepository>,
    content: Arc<dyn ContentRepository>,
    authorizer: Arc<dyn ContentMembershipAuthorizer>,
    clock: Arc<dyn Clock>,
    policy: TargetedHandoffPolicy,
}

impl TargetedHandoffService {
    pub fn new(dependencies: TargetedHandoffDependencies) -> Self {
        Self {
            store: dependencies.store,
            content: dependencies.content,
            authorizer: dependencies.authorizer,
            clock: dependencies.clock,
            policy: dependencies.policy,
        }
    }

    async fn list_targets_internal(
        &self,
        request: ListTargetedHandoffTargets,
    ) -> TargetedHandoffResult<Vec<HandoffTargetView>> {
        const OPERATION: &str = "handoff.targets.list";
        let now = self.clock.now();
        ensure_human_actor(&request.actor, now, OPERATION)?;
        self.ensure_room_access(
            request.actor.principal_id,
            request.actor.principal_id,
            &request.room_id,
            ContentAccessMode::RoomMember,
        )
        .await?;
        self.store
            .list_targets(request.actor.principal_id, now)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| HandoffTargetView {
                        online: record.instance_status == AgentInstanceStatus::Online
                            && record.lease_expires_at.is_some_and(|expiry| expiry > now),
                        record,
                    })
                    .collect()
            })
    }

    async fn create_internal(
        &self,
        request: CreateTargetedHandoff,
    ) -> TargetedHandoffResult<CreateTargetedHandoffOutcome> {
        const OPERATION: &str = "handoff.create";
        let now = self.clock.now();
        ensure_human_actor(&request.actor, now, OPERATION)?;
        self.policy.validate(now, request.expires_at)?;

        let target = self
            .store
            .list_targets(request.actor.principal_id, now)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .into_iter()
            .find(|target| target.instance_id == request.target_instance_id)
            .ok_or_else(|| failure(OPERATION, TargetedHandoffFailureKind::TargetUnavailable))?;

        let content = self
            .content
            .find_content(request.content_id)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, TargetedHandoffFailureKind::InvalidSource))?;
        let policy = self
            .content
            .find_access_policy(request.content_id)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, TargetedHandoffFailureKind::InvalidSource))?;
        if content.lifecycle_state() != ContentLifecycleState::Active
            || policy.is_revoked()
            || policy.matrix_room_id().as_str() != request.source_room_id.as_str()
            || policy
                .matrix_event_id()
                .is_none_or(|event| event.as_str() != request.source_event_id.as_str())
            || content
                .expires_at()
                .is_some_and(|expires_at| expires_at < request.expires_at)
        {
            return Err(failure(
                OPERATION,
                TargetedHandoffFailureKind::InvalidSource,
            ));
        }
        self.ensure_room_access(
            request.actor.principal_id,
            content.owner_principal_id(),
            &request.source_room_id,
            policy.access_mode(),
        )
        .await?;

        let handoff = TargetedHandoff::queue(TargetedHandoffFields {
            id: request.handoff_id,
            principal_id: request.actor.principal_id,
            source_room_id: request.source_room_id,
            source_event_id: request.source_event_id,
            source_message_id: request.source_message_id,
            target_agent_id: target.agent_id,
            target_instance_id: target.instance_id,
            content: HandoffContentReference::new(
                content.id(),
                content.digest(),
                content.byte_length(),
                content.media_type().clone(),
            ),
            permissions: request.permissions,
            purpose: request.purpose,
            created_at: now,
            expires_at: request.expires_at,
        })
        .map_err(|_| failure(OPERATION, TargetedHandoffFailureKind::InvalidRequest))?;
        let request_fingerprint = fingerprint(&handoff);
        let outcome = self
            .store
            .queue(QueueTargetedHandoff {
                handoff: &handoff,
                request_fingerprint,
            })
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?;
        Ok(match outcome {
            QueueTargetedHandoffOutcome::Created(handoff) => CreateTargetedHandoffOutcome {
                handoff,
                created: true,
            },
            QueueTargetedHandoffOutcome::Existing(handoff) => CreateTargetedHandoffOutcome {
                handoff,
                created: false,
            },
        })
    }

    async fn get_internal(
        &self,
        request: GetTargetedHandoff,
    ) -> TargetedHandoffResult<TargetedHandoff> {
        const OPERATION: &str = "handoff.get";
        let now = self.clock.now();
        ensure_human_actor(&request.actor, now, OPERATION)?;
        self.store
            .find_for_principal(request.handoff_id, request.actor.principal_id, now)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, TargetedHandoffFailureKind::NotFound))
    }

    async fn revoke_internal(
        &self,
        request: RevokeTargetedHandoff,
    ) -> TargetedHandoffResult<TargetedHandoff> {
        const OPERATION: &str = "handoff.revoke";
        let now = self.clock.now();
        ensure_human_actor(&request.actor, now, OPERATION)?;
        self.store
            .revoke(request.handoff_id, request.actor.principal_id, now)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, TargetedHandoffFailureKind::NotFound))
    }

    async fn claim_next_internal(
        &self,
        request: ClaimNextTargetedHandoff,
    ) -> TargetedHandoffResult<Option<TargetedHandoff>> {
        const OPERATION: &str = "handoff.claim";
        let now = self.clock.now();
        ensure_device_actor(&request.actor, now, OPERATION)?;
        self.store
            .claim_next(ClaimTargetedHandoff {
                principal_id: request.actor.account.principal.id(),
                device_id: request.actor.device_id,
                target_instance_id: request.target_instance_id,
                claimed_at: now,
            })
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))
    }

    async fn record_receipt_internal(
        &self,
        request: RecordTargetedHandoffReceiptCommand,
    ) -> TargetedHandoffResult<TargetedHandoff> {
        const OPERATION: &str = "handoff.receipt";
        let now = self.clock.now();
        ensure_device_actor(&request.actor, now, OPERATION)?;
        self.store
            .record_receipt(RecordTargetedHandoffReceipt {
                principal_id: request.actor.account.principal.id(),
                device_id: request.actor.device_id,
                target_instance_id: request.target_instance_id,
                handoff_id: request.handoff_id,
                outcome: request.outcome,
                recorded_at: now,
            })
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, TargetedHandoffFailureKind::NotFound))
    }

    async fn ensure_room_access(
        &self,
        principal_id: agent_room_domain::ids::PrincipalId,
        owner_principal_id: agent_room_domain::ids::PrincipalId,
        room_id: &MatrixRoomReference,
        access_mode: ContentAccessMode,
    ) -> TargetedHandoffResult<()> {
        let matrix_room_id = MatrixRoomId::new(room_id.as_str().to_owned()).map_err(|_| {
            failure(
                "handoff.room_authorize",
                TargetedHandoffFailureKind::InvalidRequest,
            )
        })?;
        let decision = self
            .authorizer
            .authorize(&ContentAuthorizationRequest {
                principal_id,
                actor_agent_id: None,
                owner_principal_id,
                matrix_room_id,
                access_mode,
                intent: ContentAuthorizationIntent::Read,
            })
            .await
            .map_err(|error| {
                let kind = match error.kind() {
                    ContentAuthorizationFailureKind::Denied => {
                        TargetedHandoffFailureKind::Forbidden
                    }
                    ContentAuthorizationFailureKind::StaleProjection
                    | ContentAuthorizationFailureKind::Unavailable => {
                        TargetedHandoffFailureKind::DependencyUnavailable
                    }
                };
                failure("handoff.room_authorize", kind)
            })?;
        match decision {
            ContentAuthorizationDecision::Allowed => Ok(()),
            ContentAuthorizationDecision::Denied => Err(failure(
                "handoff.room_authorize",
                TargetedHandoffFailureKind::Forbidden,
            )),
        }
    }
}

impl TargetedHandoffUseCases for TargetedHandoffService {
    fn list_targets(
        &self,
        request: ListTargetedHandoffTargets,
    ) -> PortFuture<'_, TargetedHandoffResult<Vec<HandoffTargetView>>> {
        Box::pin(self.list_targets_internal(request))
    }

    fn create(
        &self,
        request: CreateTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<CreateTargetedHandoffOutcome>> {
        Box::pin(self.create_internal(request))
    }

    fn get(
        &self,
        request: GetTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<TargetedHandoff>> {
        Box::pin(self.get_internal(request))
    }

    fn revoke(
        &self,
        request: RevokeTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<TargetedHandoff>> {
        Box::pin(self.revoke_internal(request))
    }

    fn claim_next(
        &self,
        request: ClaimNextTargetedHandoff,
    ) -> PortFuture<'_, TargetedHandoffResult<Option<TargetedHandoff>>> {
        Box::pin(self.claim_next_internal(request))
    }

    fn record_receipt(
        &self,
        request: RecordTargetedHandoffReceiptCommand,
    ) -> PortFuture<'_, TargetedHandoffResult<TargetedHandoff>> {
        Box::pin(self.record_receipt_internal(request))
    }
}

fn fingerprint(handoff: &TargetedHandoff) -> TargetedHandoffRequestFingerprint {
    let fields = handoff.fields();
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, fields.principal_id.as_uuid().as_bytes());
    hash_field(&mut hasher, fields.source_room_id.as_str().as_bytes());
    hash_field(&mut hasher, fields.source_event_id.as_str().as_bytes());
    hash_field(&mut hasher, fields.source_message_id.as_uuid().as_bytes());
    hash_field(&mut hasher, fields.target_agent_id.as_uuid().as_bytes());
    hash_field(&mut hasher, fields.target_instance_id.as_uuid().as_bytes());
    hash_field(
        &mut hasher,
        fields.content.content_id().as_uuid().as_bytes(),
    );
    hash_field(&mut hasher, fields.content.digest().as_bytes());
    hash_field(
        &mut hasher,
        &fields.content.byte_length().value().to_be_bytes(),
    );
    hash_field(&mut hasher, fields.content.media_type().as_str().as_bytes());
    for permission in fields.permissions.iter() {
        hash_field(&mut hasher, permission.as_str().as_bytes());
    }
    hash_field(&mut hasher, fields.purpose.as_str().as_bytes());
    hash_field(&mut hasher, &fields.expires_at.value().to_be_bytes());
    TargetedHandoffRequestFingerprint::from_bytes(hasher.finalize().into())
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn ensure_human_actor(
    actor: &AuthenticatedPrincipal,
    now: UtcMillis,
    operation: &'static str,
) -> TargetedHandoffResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(operation, TargetedHandoffFailureKind::Unauthorized))
    }
}

fn ensure_device_actor(
    actor: &AuthenticatedDevice,
    now: UtcMillis,
    operation: &'static str,
) -> TargetedHandoffResult<()> {
    if actor.account.principal.allows_authentication() && now < actor.access_token_expires_at {
        Ok(())
    } else {
        Err(failure(operation, TargetedHandoffFailureKind::Unauthorized))
    }
}

const fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> TargetedHandoffFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Forbidden => TargetedHandoffFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => TargetedHandoffFailureKind::NotFound,
        RepositoryErrorKind::Conflict => TargetedHandoffFailureKind::Conflict,
        RepositoryErrorKind::Unavailable => TargetedHandoffFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            TargetedHandoffFailureKind::Internal
        }
    };
    failure(operation, kind)
}

const fn failure(
    operation: &'static str,
    kind: TargetedHandoffFailureKind,
) -> TargetedHandoffFailure {
    TargetedHandoffFailure::new(operation, kind)
}
