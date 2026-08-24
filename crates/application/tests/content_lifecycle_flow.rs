use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    content::{
        BindContentEventDependencies, BindContentEventFailure, BindContentEventOutcome,
        BindContentEventRequest, BindContentEventService, CleanupContentDependencies,
        CleanupContentItemFailureCause, CleanupContentPolicy, CleanupContentService,
        ContentCleanupStage, RedactContentDependencies, RedactContentFailure, RedactContentOutcome,
        RedactContentRequest, RedactContentService,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, ContentAccessMode, ContentAccessPolicy, ContentByteStream, ContentEventBinding,
        ContentLifecycleTransition, ContentRepository, ContentUploadClaim,
        ContentUploadClaimOutcome, MatrixEventId, MatrixRoomId, ObjectStoreFailure,
        ObjectStoreFailureKind, ObjectStoreResult, ObjectWriteReceipt, OpenedContentObject,
        PortFuture, PrivateContentObjectStore, ReclaimableContentQuery,
    },
};
use agent_room_domain::{
    DomainError,
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    ids::{ContentId, PrincipalId},
    time::UtcMillis,
};
use uuid::Uuid;

#[tokio::test]
async fn 已发送事件被幂等绑定且不能换绑到另一个事件() {
    let owner = PrincipalId::from_uuid(Uuid::now_v7());
    let content = content(owner, ContentLifecycleState::Active, Some(time(50_000)));
    let room_id = room_id(content.id());
    let repository = Arc::new(MemoryLifecycleRepository::new([content.clone()]));
    repository.insert_policy(policy(&content, room_id.clone()));
    let service = BindContentEventService::new(BindContentEventDependencies {
        clock: Arc::new(FixedClock),
        repository: repository.clone(),
    });
    let event_id = MatrixEventId::new("$bound-event").expect("事件 ID 有效");
    let request = BindContentEventRequest {
        principal_id: owner,
        content_id: content.id(),
        matrix_room_id: room_id.clone(),
        matrix_event_id: event_id.clone(),
    };

    assert!(matches!(
        service.bind(request.clone()).await.expect("首次绑定成功"),
        BindContentEventOutcome::Bound(_)
    ));
    assert!(matches!(
        service.bind(request).await.expect("重复绑定幂等成功"),
        BindContentEventOutcome::AlreadyBound(_)
    ));
    let failure = service
        .bind(BindContentEventRequest {
            principal_id: owner,
            content_id: content.id(),
            matrix_room_id: room_id,
            matrix_event_id: MatrixEventId::new("$different-event").expect("事件 ID 有效"),
        })
        .await
        .expect_err("已经绑定的内容不能换绑");
    assert_eq!(failure, BindContentEventFailure::EventConflict);
}

#[tokio::test]
async fn 所有者撤回正文立即关闭读取状态且重复请求幂等() {
    let owner = PrincipalId::from_uuid(Uuid::now_v7());
    let stranger = PrincipalId::from_uuid(Uuid::now_v7());
    let content = content(owner, ContentLifecycleState::Active, Some(time(50_000)));
    let repository = Arc::new(MemoryLifecycleRepository::new([content.clone()]));
    let service = RedactContentService::new(RedactContentDependencies {
        clock: Arc::new(FixedClock),
        repository: repository.clone(),
    });

    let forbidden = service
        .redact(RedactContentRequest {
            principal_id: stranger,
            content_id: content.id(),
        })
        .await
        .expect_err("非所有者不能撤回正文");
    assert_eq!(forbidden, RedactContentFailure::Forbidden);

    let request = RedactContentRequest {
        principal_id: owner,
        content_id: content.id(),
    };
    assert!(matches!(
        service.redact(request).await.expect("首次撤回成功"),
        RedactContentOutcome::Redacted(_)
    ));
    assert!(matches!(
        service.redact(request).await.expect("重复撤回幂等成功"),
        RedactContentOutcome::AlreadyRedacted(_)
    ));
    assert_eq!(
        repository.state(content.id()),
        ContentLifecycleState::Redacted
    );
}

#[tokio::test]
async fn 回收批次区分孤儿与过期对象且单项失败不阻塞其余对象() {
    let owner = PrincipalId::from_uuid(Uuid::now_v7());
    let stale_upload = content(owner, ContentLifecycleState::Uploading, Some(time(50_000)));
    let unbound_active = content(owner, ContentLifecycleState::Active, Some(time(50_000)));
    let expired_active = content(owner, ContentLifecycleState::Active, Some(time(9_000)));
    let failed_orphan = content(owner, ContentLifecycleState::Orphaned, Some(time(50_000)));
    let repository = Arc::new(MemoryLifecycleRepository::new([
        stale_upload.clone(),
        unbound_active.clone(),
        expired_active.clone(),
        failed_orphan.clone(),
    ]));
    let object_store = Arc::new(RecordingObjectStore::failing(failed_orphan.id()));
    let service = CleanupContentService::new(CleanupContentDependencies {
        clock: Arc::new(FixedClock),
        repository: repository.clone(),
        object_store: object_store.clone(),
        policy: CleanupContentPolicy::new(1_000, 20).expect("回收策略有效"),
    });

    let outcome = service.run().await.expect("批次查询成功");

    assert_eq!(outcome.examined, 4);
    assert_eq!(outcome.deleted, 3);
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(outcome.failures[0].content_id, failed_orphan.id());
    assert_eq!(outcome.failures[0].stage, ContentCleanupStage::DeleteObject);
    assert!(matches!(
        outcome.failures[0].cause,
        CleanupContentItemFailureCause::ObjectStore(_)
    ));
    assert_eq!(
        repository.state(stale_upload.id()),
        ContentLifecycleState::Deleted
    );
    assert_eq!(
        repository.state(unbound_active.id()),
        ContentLifecycleState::Deleted
    );
    assert_eq!(
        repository.state(expired_active.id()),
        ContentLifecycleState::Deleted
    );
    assert_eq!(
        repository.state(failed_orphan.id()),
        ContentLifecycleState::Orphaned
    );
    let transition_targets = repository.transition_targets();
    assert_eq!(
        transition_targets
            .iter()
            .filter(|target| **target == ContentLifecycleState::Orphaned)
            .count(),
        2
    );
    assert_eq!(
        transition_targets
            .iter()
            .filter(|target| **target == ContentLifecycleState::Expired)
            .count(),
        1
    );
    assert_eq!(object_store.deleted_count(), 3);
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        time(10_000)
    }
}

struct MemoryLifecycleRepository {
    state: Mutex<RepositoryState>,
}

struct RepositoryState {
    contents: HashMap<ContentId, ContentObject>,
    policies: HashMap<ContentId, ContentAccessPolicy>,
    transition_targets: Vec<ContentLifecycleState>,
}

impl MemoryLifecycleRepository {
    fn new(contents: impl IntoIterator<Item = ContentObject>) -> Self {
        Self {
            state: Mutex::new(RepositoryState {
                contents: contents
                    .into_iter()
                    .map(|content| (content.id(), content))
                    .collect(),
                policies: HashMap::new(),
                transition_targets: Vec::new(),
            }),
        }
    }

    fn insert_policy(&self, policy: ContentAccessPolicy) {
        self.state
            .lock()
            .expect("仓储锁有效")
            .policies
            .insert(policy.content_id(), policy);
    }

    fn state(&self, content_id: ContentId) -> ContentLifecycleState {
        self.state
            .lock()
            .expect("仓储锁有效")
            .contents
            .get(&content_id)
            .expect("内容存在")
            .lifecycle_state()
    }

    fn transition_targets(&self) -> Vec<ContentLifecycleState> {
        self.state
            .lock()
            .expect("仓储锁有效")
            .transition_targets
            .clone()
    }

    fn mutate_content(
        &self,
        content_id: ContentId,
        operation: impl FnOnce(&mut ContentObject) -> Result<(), DomainError>,
    ) -> RepositoryResult<ContentObject> {
        let mut state = self.state.lock().expect("仓储锁有效");
        let content = state.contents.get_mut(&content_id).ok_or_else(|| {
            RepositoryError::new("content.test.mutate", RepositoryErrorKind::NotFound)
        })?;
        operation(content).map_err(|_| {
            RepositoryError::new("content.test.mutate", RepositoryErrorKind::Constraint)
        })?;
        Ok(content.clone())
    }
}

impl ContentRepository for MemoryLifecycleRepository {
    fn claim_upload<'a>(
        &'a self,
        _claim: &'a ContentUploadClaim,
    ) -> PortFuture<'a, RepositoryResult<ContentUploadClaimOutcome>> {
        unsupported_repository()
    }

    fn find_content(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentObject>>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .expect("仓储锁有效")
                .contents
                .get(&content_id)
                .cloned())
        })
    }

    fn find_access_policy(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentAccessPolicy>>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .expect("仓储锁有效")
                .policies
                .get(&content_id)
                .cloned())
        })
    }

    fn activate(
        &self,
        _content_id: ContentId,
        _activated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }

    fn record_scan(
        &self,
        _content_id: ContentId,
        _outcome: ContentScanState,
        _scanned_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }

    fn bind_event<'a>(
        &'a self,
        binding: &'a ContentEventBinding,
    ) -> PortFuture<'a, RepositoryResult<ContentAccessPolicy>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("仓储锁有效");
            let policy = state.policies.get(&binding.content_id).ok_or_else(|| {
                RepositoryError::new("content.test.bind", RepositoryErrorKind::NotFound)
            })?;
            let bound = ContentAccessPolicy::restore(
                policy.content_id(),
                policy.matrix_room_id().clone(),
                Some(binding.matrix_event_id.clone()),
                policy.access_mode(),
                policy.created_at(),
                policy.revoked_at(),
            )
            .map_err(|_| {
                RepositoryError::new("content.test.bind", RepositoryErrorKind::Constraint)
            })?;
            state.policies.insert(binding.content_id, bound.clone());
            Ok(bound)
        })
    }

    fn transition<'a>(
        &'a self,
        transition: &'a ContentLifecycleTransition,
    ) -> PortFuture<'a, RepositoryResult<ContentObject>> {
        Box::pin(async move {
            let result = self.mutate_content(transition.content_id, |content| {
                if content.lifecycle_state() != transition.expected {
                    return Err(DomainError::InvalidTransition {
                        entity: "content_object",
                        from: content.lifecycle_state().as_str(),
                        to: transition.target.as_str(),
                    });
                }
                match transition.target {
                    ContentLifecycleState::Orphaned => content.mark_orphaned(),
                    ContentLifecycleState::Expired => content.expire(transition.changed_at),
                    ContentLifecycleState::Redacted => content.redact(),
                    ContentLifecycleState::Deleted => content.mark_deleted(transition.changed_at),
                    ContentLifecycleState::Active => content.activate(),
                    ContentLifecycleState::Uploading => Err(DomainError::InvalidTransition {
                        entity: "content_object",
                        from: content.lifecycle_state().as_str(),
                        to: ContentLifecycleState::Uploading.as_str(),
                    }),
                }
            })?;
            self.state
                .lock()
                .expect("仓储锁有效")
                .transition_targets
                .push(transition.target);
            Ok(result)
        })
    }

    fn list_reclaimable<'a>(
        &'a self,
        _query: &'a ReclaimableContentQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<ContentObject>>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .expect("仓储锁有效")
                .contents
                .values()
                .cloned()
                .collect())
        })
    }

    fn mark_deleted(
        &self,
        content_id: ContentId,
        deleted_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        Box::pin(async move {
            self.mutate_content(content_id, |content| content.mark_deleted(deleted_at))
        })
    }
}

fn unsupported_repository<'a, T>() -> PortFuture<'a, RepositoryResult<T>> {
    Box::pin(async {
        Err(RepositoryError::new(
            "content.test.unsupported",
            RepositoryErrorKind::Unavailable,
        ))
    })
}

struct RecordingObjectStore {
    failing_content_id: ContentId,
    deleted: Mutex<Vec<ContentId>>,
}

impl RecordingObjectStore {
    const fn failing(failing_content_id: ContentId) -> Self {
        Self {
            failing_content_id,
            deleted: Mutex::new(Vec::new()),
        }
    }

    fn deleted_count(&self) -> usize {
        self.deleted.lock().expect("删除记录锁有效").len()
    }
}

impl PrivateContentObjectStore for RecordingObjectStore {
    fn put<'a>(
        &'a self,
        _content: &'a ContentObject,
        _body: ContentByteStream,
    ) -> PortFuture<'a, ObjectStoreResult<ObjectWriteReceipt>> {
        unsupported_object_store()
    }

    fn open<'a>(
        &'a self,
        _content: &'a ContentObject,
    ) -> PortFuture<'a, ObjectStoreResult<OpenedContentObject>> {
        unsupported_object_store()
    }

    fn delete<'a>(&'a self, content: &'a ContentObject) -> PortFuture<'a, ObjectStoreResult<()>> {
        Box::pin(async move {
            if content.id() == self.failing_content_id {
                return Err(ObjectStoreFailure::new(
                    "content.test.delete",
                    ObjectStoreFailureKind::Unavailable,
                ));
            }
            self.deleted
                .lock()
                .expect("删除记录锁有效")
                .push(content.id());
            Ok(())
        })
    }
}

fn unsupported_object_store<'a, T>() -> PortFuture<'a, ObjectStoreResult<T>> {
    Box::pin(async {
        Err(ObjectStoreFailure::new(
            "content.test.unsupported",
            ObjectStoreFailureKind::Unavailable,
        ))
    })
}

fn content(
    owner_principal_id: PrincipalId,
    state: ContentLifecycleState,
    expires_at: Option<UtcMillis>,
) -> ContentObject {
    let content_id = ContentId::from_uuid(Uuid::now_v7());
    let mut content = ContentObject::begin_upload(ContentObjectFields {
        id: content_id,
        owner_principal_id,
        storage_key: ContentStorageKey::new(format!("content/{content_id}/opaque-random-suffix"))
            .expect("对象键有效"),
        digest: Sha256Digest::from_bytes([0x42; 32]),
        byte_length: ContentByteLength::new(42).expect("长度有效"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode: ContentEncryptionMode::ServerSide,
        scan_state: ContentScanState::Clean,
        lifecycle_state: ContentLifecycleState::Uploading,
        expires_at,
        created_at: time(1_000),
        deleted_at: None,
    })
    .expect("内容有效");
    match state {
        ContentLifecycleState::Uploading => {}
        ContentLifecycleState::Active => content.activate().expect("可激活"),
        ContentLifecycleState::Orphaned => content.mark_orphaned().expect("可转孤儿"),
        _ => panic!("测试构造器不支持状态 {state:?}"),
    }
    content
}

fn policy(content: &ContentObject, room_id: MatrixRoomId) -> ContentAccessPolicy {
    ContentAccessPolicy::new(
        content.id(),
        room_id,
        ContentAccessMode::RoomMember,
        time(1_000),
    )
}

fn room_id(content_id: ContentId) -> MatrixRoomId {
    MatrixRoomId::new(format!("!room-{content_id}:example.test")).expect("房间 ID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
