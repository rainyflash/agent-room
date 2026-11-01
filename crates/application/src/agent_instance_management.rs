use std::sync::Arc;

use agent_room_domain::{agents::AgentInstanceStatus, ids::AgentInstanceId, time::UtcMillis};
use futures_util::stream::{self, StreamExt as _};

use crate::{
    authentication::AuthenticatedPrincipal,
    matrix_device_cleanup::{MatrixDeviceCleanupFailure, revoke_agent_matrix_device},
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentInstanceManagementRecord, AgentInstanceManagementRepository,
        AgentInstanceMatrixCleanupStore, AgentInstanceRevocationOutcome,
        AgentInstanceRevocationTransaction, Clock, MatrixAgentDeviceActivityReader,
        MatrixAgentDeviceSessionRevoker, MatrixAgentDeviceSessionTarget, MatrixDeviceId,
        MatrixFailureKind, MatrixUserId, PortFuture,
    },
};

/// 一次最多问几个连接的设备：一个账户攒下的连接再多也不至于这么多，超出的不问。
const MAX_ACTIVITY_LOOKUPS: usize = 500;
/// 同时问 Synapse 的个数。
const ACTIVITY_CONCURRENCY: usize = 8;

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

/// 一个还在用的连接，它的 Matrix 设备上次用是什么时候（`specs/workspace-tidy/design.md`“30 天没上线的
/// Agent”）。连接上记的 `last_seen_at` 只在控制后台回复时写，看不出 Agent 上没上线；Synapse 记的是这台
/// 设备最后一次发请求，等消息、发言、报在线状态都算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentInstanceMatrixActivity {
    pub instance_id: AgentInstanceId,
    /// 设备已经不在了、或者 Synapse 没记过时为 `None`。
    pub last_seen_at: Option<UtcMillis>,
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

    /// 每个还在用的连接的设备上次用的时间。问不到的连接不在结果里；一个都问不到时报依赖不可用。
    fn matrix_activity(
        &self,
        request: ListAgentInstances,
    ) -> PortFuture<'_, AgentInstanceManagementResult<Vec<AgentInstanceMatrixActivity>>>;
}

pub struct AgentInstanceManagementService {
    instances: Arc<dyn AgentInstanceManagementRepository>,
    revocations: Arc<dyn AgentInstanceRevocationTransaction>,
    matrix_cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    matrix_activity: Arc<dyn MatrixAgentDeviceActivityReader>,
    clock: Arc<dyn Clock>,
}

pub struct AgentInstanceManagementDependencies {
    pub instances: Arc<dyn AgentInstanceManagementRepository>,
    pub revocations: Arc<dyn AgentInstanceRevocationTransaction>,
    pub matrix_cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    pub matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    pub matrix_activity: Arc<dyn MatrixAgentDeviceActivityReader>,
    pub clock: Arc<dyn Clock>,
}

impl AgentInstanceManagementService {
    pub fn new(dependencies: AgentInstanceManagementDependencies) -> Self {
        Self {
            instances: dependencies.instances,
            revocations: dependencies.revocations,
            matrix_cleanup: dependencies.matrix_cleanup,
            matrix: dependencies.matrix,
            matrix_activity: dependencies.matrix_activity,
            clock: dependencies.clock,
        }
    }

    async fn matrix_activity_internal(
        &self,
        request: ListAgentInstances,
    ) -> AgentInstanceManagementResult<Vec<AgentInstanceMatrixActivity>> {
        let operation = "agent_instance.matrix_activity";
        ensure_active_actor(&request.actor, self.clock.now(), operation)?;
        let records = self
            .instances
            .list_for_principal(request.actor.principal_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        // 先拿成自己的数据再并发去问：借着 `records` 的闭包放进 `buffer_unordered`，整个 future
        // 就证明不了对任意生命周期都 `Send`。
        let targets: Vec<(AgentInstanceId, MatrixAgentDeviceSessionTarget)> = records
            .iter()
            .filter(|record| {
                record.revoked_at.is_none() && record.matrix_device_revoked_at.is_none()
            })
            .take(MAX_ACTIVITY_LOOKUPS)
            .filter_map(activity_target)
            .collect();
        let asked = targets.len();
        let answers: Vec<Option<AgentInstanceMatrixActivity>> = stream::iter(targets)
            .map(|(instance_id, target)| async move {
                let last_seen_at = self.matrix_activity.device_last_seen(&target).await.ok()?;
                Some(AgentInstanceMatrixActivity {
                    instance_id,
                    last_seen_at,
                })
            })
            .buffer_unordered(ACTIVITY_CONCURRENCY)
            .collect()
            .await;
        if asked > 0 && answers.iter().all(Option::is_none) {
            return Err(failure(
                operation,
                AgentInstanceManagementFailureKind::DependencyUnavailable,
            ));
        }
        Ok(answers.into_iter().flatten().collect())
    }

    async fn list_instances_internal(
        &self,
        request: ListAgentInstances,
    ) -> AgentInstanceManagementResult<Vec<AgentInstanceManagementRecord>> {
        let operation = "agent_instance.list";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, operation)?;
        let mut records = self
            .instances
            .list_for_principal(request.actor.principal_id)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;
        for record in &mut records {
            settle_status(record, now);
        }
        Ok(records)
    }

    async fn revoke_instance_internal(
        &self,
        request: RevokeAgentInstance,
    ) -> AgentInstanceManagementResult<RevokedAgentInstance> {
        let operation = "agent_instance.revoke";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, operation)?;
        let outcome = self
            .revocations
            .revoke(request.actor.principal_id, request.instance_id, now)
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

/// “正在连接”只该是刚登记的那一会儿：登记以后这么久还没续上的，按离线报。和在线租约一样长。
const CONNECTING_WINDOW_MS: i64 = 90_000;

/// 库里的状态只在控制后台回复、自动发言时写成在线（带 90 秒租约），断开时没人改回来，只用 MCP
/// 或命令行等消息的从来不续、一直是正在连接。所以列连接时按时间算：租约过了的、登记以后一直没续上
/// 的都按离线报，“上次在线”照旧是最后那次活动。库里不动。在不在线以 Matrix 的在线状态为准
/// （`specs/agent-liveness/design.md`），这里只是不再报一个早就不对的状态。
fn activity_target(
    record: &AgentInstanceManagementRecord,
) -> Option<(AgentInstanceId, MatrixAgentDeviceSessionTarget)> {
    let user_id = MatrixUserId::new(record.agent_matrix_user_id.clone()).ok()?;
    let device_id =
        MatrixDeviceId::new(record.instance.matrix_device_id().as_str().to_owned()).ok()?;
    Some((
        record.instance.id(),
        MatrixAgentDeviceSessionTarget::new(user_id, device_id),
    ))
}

fn settle_status(record: &mut AgentInstanceManagementRecord, now: UtcMillis) {
    record.instance.expire(now);
    let since = record.last_seen_at.unwrap_or(record.created_at);
    if record.instance.status() == AgentInstanceStatus::Connecting
        && now.value().saturating_sub(since.value()) >= CONNECTING_WINDOW_MS
    {
        record.instance.disconnect();
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

    fn matrix_activity(
        &self,
        request: ListAgentInstances,
    ) -> PortFuture<'_, AgentInstanceManagementResult<Vec<AgentInstanceMatrixActivity>>> {
        Box::pin(self.matrix_activity_internal(request))
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
