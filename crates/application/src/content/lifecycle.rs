use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    content::{ContentLifecycleState, ContentObject},
    ids::{ContentId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::RepositoryError,
    ports::{
        Clock, ContentAccessPolicy, ContentEventBinding, ContentLifecycleTransition,
        ContentRepository, MatrixEventId, MatrixRoomId, ObjectStoreFailure, ObjectStoreFailureKind,
        PortFuture, PrivateContentObjectStore, ReclaimableContentQuery,
    },
};

const MAXIMUM_ORPHAN_GRACE_MILLIS: u64 = 24 * 60 * 60 * 1_000;
const MAXIMUM_CLEANUP_BATCH: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindContentEventRequest {
    pub principal_id: PrincipalId,
    pub content_id: ContentId,
    pub matrix_room_id: MatrixRoomId,
    pub matrix_event_id: MatrixEventId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindContentEventOutcome {
    Bound(ContentAccessPolicy),
    AlreadyBound(ContentAccessPolicy),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindContentEventFailure {
    NotFound,
    Forbidden,
    InvalidState(ContentLifecycleState),
    Revoked,
    PolicyMismatch,
    EventConflict,
    Repository(RepositoryError),
}

pub type BindContentEventResult<T> = Result<T, BindContentEventFailure>;

pub struct BindContentEventDependencies {
    pub clock: Arc<dyn Clock>,
    pub repository: Arc<dyn ContentRepository>,
}

pub struct BindContentEventService {
    clock: Arc<dyn Clock>,
    repository: Arc<dyn ContentRepository>,
}

impl BindContentEventService {
    pub fn new(dependencies: BindContentEventDependencies) -> Self {
        Self {
            clock: dependencies.clock,
            repository: dependencies.repository,
        }
    }

    /// 将已成功发送的 Matrix 事件幂等绑定到活跃内容。
    ///
    /// # Errors
    ///
    /// 内容不存在、调用主体不是所有者、策略已撤销、房间或事件冲突以及仓储失败时返回错误。
    pub async fn bind(
        &self,
        request: BindContentEventRequest,
    ) -> BindContentEventResult<BindContentEventOutcome> {
        let content = self
            .repository
            .find_content(request.content_id)
            .await
            .map_err(BindContentEventFailure::Repository)?
            .ok_or(BindContentEventFailure::NotFound)?;
        if content.owner_principal_id() != request.principal_id {
            return Err(BindContentEventFailure::Forbidden);
        }
        if content.lifecycle_state() != ContentLifecycleState::Active {
            return Err(BindContentEventFailure::InvalidState(
                content.lifecycle_state(),
            ));
        }
        let policy = self
            .repository
            .find_access_policy(request.content_id)
            .await
            .map_err(BindContentEventFailure::Repository)?
            .ok_or(BindContentEventFailure::NotFound)?;
        if policy.is_revoked() {
            return Err(BindContentEventFailure::Revoked);
        }
        if policy.matrix_room_id() != &request.matrix_room_id {
            return Err(BindContentEventFailure::PolicyMismatch);
        }
        match policy.matrix_event_id() {
            Some(event_id) if event_id == &request.matrix_event_id => {
                return Ok(BindContentEventOutcome::AlreadyBound(policy));
            }
            Some(_) => return Err(BindContentEventFailure::EventConflict),
            None => {}
        }
        let bound = self
            .repository
            .bind_event(&ContentEventBinding {
                content_id: request.content_id,
                matrix_room_id: request.matrix_room_id,
                matrix_event_id: request.matrix_event_id,
                bound_at: self.clock.now(),
            })
            .await
            .map_err(BindContentEventFailure::Repository)?;
        Ok(BindContentEventOutcome::Bound(bound))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupContentPolicy {
    orphan_grace: DurationMillis,
    batch_limit: u16,
}

impl CleanupContentPolicy {
    /// 创建后台内容回收策略。
    ///
    /// # Errors
    ///
    /// 孤儿宽限期不在一天以内或批大小不在 1 到 500 之间时返回错误。
    pub fn new(orphan_grace_millis: u64, batch_limit: u16) -> Result<Self, DomainError> {
        if orphan_grace_millis > MAXIMUM_ORPHAN_GRACE_MILLIS {
            return Err(DomainError::Validation {
                field: "content_orphan_grace",
                reason: "必须在一天以内",
            });
        }
        if !(1..=MAXIMUM_CLEANUP_BATCH).contains(&batch_limit) {
            return Err(DomainError::Validation {
                field: "content_cleanup_batch_limit",
                reason: "必须在 1 到 500 之间",
            });
        }
        Ok(Self {
            orphan_grace: DurationMillis::new(orphan_grace_millis)?,
            batch_limit,
        })
    }

    const fn orphan_grace(self) -> DurationMillis {
        self.orphan_grace
    }

    const fn batch_limit(self) -> u16 {
        self.batch_limit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentCleanupStage {
    Transition,
    DeleteObject,
    MarkDeleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupContentItemFailureCause {
    Repository(RepositoryError),
    ObjectStore(ObjectStoreFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupContentItemFailure {
    pub content_id: ContentId,
    pub stage: ContentCleanupStage,
    pub cause: CleanupContentItemFailureCause,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupContentOutcome {
    pub examined: u16,
    pub deleted: u16,
    pub failures: Vec<CleanupContentItemFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupContentFailure {
    Repository(RepositoryError),
}

pub type CleanupContentResult<T> = Result<T, CleanupContentFailure>;

/// 后台执行器依赖的内容回收能力边界。
pub trait ContentCleanupUseCases: Send + Sync {
    fn run_cleanup(&self) -> PortFuture<'_, CleanupContentResult<CleanupContentOutcome>>;
}

pub struct CleanupContentDependencies {
    pub clock: Arc<dyn Clock>,
    pub repository: Arc<dyn ContentRepository>,
    pub object_store: Arc<dyn PrivateContentObjectStore>,
    pub policy: CleanupContentPolicy,
}

pub struct CleanupContentService {
    clock: Arc<dyn Clock>,
    repository: Arc<dyn ContentRepository>,
    object_store: Arc<dyn PrivateContentObjectStore>,
    policy: CleanupContentPolicy,
}

impl CleanupContentService {
    pub fn new(dependencies: CleanupContentDependencies) -> Self {
        Self {
            clock: dependencies.clock,
            repository: dependencies.repository,
            object_store: dependencies.object_store,
            policy: dependencies.policy,
        }
    }

    /// 分批回收卡死上传、未绑定事件、撤回和过期内容。
    ///
    /// # Errors
    ///
    /// 候选查询失败时终止本批；单个对象的状态、存储或终态写入失败会保留在结果中并继续处理。
    pub async fn run(&self) -> CleanupContentResult<CleanupContentOutcome> {
        let now = self.clock.now();
        let orphaned_before = subtract_duration(now, self.policy.orphan_grace());
        let candidates = self
            .repository
            .list_reclaimable(&ReclaimableContentQuery {
                now,
                orphaned_before,
                limit: self.policy.batch_limit(),
            })
            .await
            .map_err(CleanupContentFailure::Repository)?;
        let examined = u16::try_from(candidates.len()).unwrap_or(self.policy.batch_limit());
        let mut deleted = 0_u16;
        let mut failures = Vec::new();
        for candidate in candidates {
            match self.cleanup_one(candidate, now).await {
                Ok(()) => deleted = deleted.saturating_add(1),
                Err(failure) => failures.push(failure),
            }
        }
        Ok(CleanupContentOutcome {
            examined,
            deleted,
            failures,
        })
    }

    async fn cleanup_one(
        &self,
        candidate: ContentObject,
        now: UtcMillis,
    ) -> Result<(), CleanupContentItemFailure> {
        let prepared = self.prepare_terminal(candidate, now).await?;
        match self.object_store.delete(&prepared).await {
            Ok(()) => {}
            Err(error) if error.kind() == ObjectStoreFailureKind::NotFound => {}
            Err(error) => {
                return Err(item_failure(
                    prepared.id(),
                    ContentCleanupStage::DeleteObject,
                    CleanupContentItemFailureCause::ObjectStore(error),
                ));
            }
        }
        self.repository
            .mark_deleted(prepared.id(), now)
            .await
            .map_err(|error| {
                item_failure(
                    prepared.id(),
                    ContentCleanupStage::MarkDeleted,
                    CleanupContentItemFailureCause::Repository(error),
                )
            })?;
        Ok(())
    }

    async fn prepare_terminal(
        &self,
        candidate: ContentObject,
        now: UtcMillis,
    ) -> Result<ContentObject, CleanupContentItemFailure> {
        let target = match candidate.lifecycle_state() {
            ContentLifecycleState::Active
                if candidate
                    .expires_at()
                    .is_some_and(|expires_at| expires_at <= now) =>
            {
                Some(ContentLifecycleState::Expired)
            }
            ContentLifecycleState::Uploading | ContentLifecycleState::Active => {
                Some(ContentLifecycleState::Orphaned)
            }
            ContentLifecycleState::Orphaned
            | ContentLifecycleState::Redacted
            | ContentLifecycleState::Expired => None,
            ContentLifecycleState::Deleted => return Ok(candidate),
        };
        let Some(target) = target else {
            return Ok(candidate);
        };
        self.repository
            .transition(&ContentLifecycleTransition {
                content_id: candidate.id(),
                expected: candidate.lifecycle_state(),
                target,
                changed_at: now,
            })
            .await
            .map_err(|error| {
                item_failure(
                    candidate.id(),
                    ContentCleanupStage::Transition,
                    CleanupContentItemFailureCause::Repository(error),
                )
            })
    }
}

impl ContentCleanupUseCases for CleanupContentService {
    fn run_cleanup(&self) -> PortFuture<'_, CleanupContentResult<CleanupContentOutcome>> {
        Box::pin(self.run())
    }
}

fn subtract_duration(now: UtcMillis, duration: DurationMillis) -> UtcMillis {
    let duration = i64::try_from(duration.value()).unwrap_or(i64::MAX);
    UtcMillis::new(now.value().saturating_sub(duration).max(0))
        .expect("非负饱和时间必须满足领域约束")
}

const fn item_failure(
    content_id: ContentId,
    stage: ContentCleanupStage,
    cause: CleanupContentItemFailureCause,
) -> CleanupContentItemFailure {
    CleanupContentItemFailure {
        content_id,
        stage,
        cause,
    }
}

#[cfg(test)]
mod tests {
    use agent_room_domain::DomainError;

    use super::{CleanupContentPolicy, subtract_duration};
    use agent_room_domain::time::{DurationMillis, UtcMillis};

    #[test]
    fn 回收策略拒绝零宽限期和失控批大小() {
        assert!(matches!(
            CleanupContentPolicy::new(0, 10),
            Err(DomainError::Validation { .. })
        ));
        assert!(CleanupContentPolicy::new(1, 500).is_ok());
        assert!(CleanupContentPolicy::new(1, 501).is_err());
    }

    #[test]
    fn 宽限期回推在纪元处饱和() {
        let now = UtcMillis::new(10).expect("时间有效");
        let duration = DurationMillis::new(20).expect("时长有效");
        assert_eq!(subtract_duration(now, duration).value(), 0);
    }
}
