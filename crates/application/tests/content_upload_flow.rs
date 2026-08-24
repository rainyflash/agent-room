use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    content::{
        BeginContentUploadDependencies, BeginContentUploadFailure, BeginContentUploadOutcome,
        BeginContentUploadRequest, BeginContentUploadService, CompleteContentUploadDependencies,
        CompleteContentUploadFailure, CompleteContentUploadOutcome, CompleteContentUploadRequest,
        CompleteContentUploadService, ContentIdentifierFactory,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, ContentAccessMode, ContentAccessPolicy, ContentAuthorizationDecision,
        ContentAuthorizationRequest, ContentAuthorizationResult, ContentByteStream,
        ContentEventBinding, ContentLifecycleTransition, ContentMembershipAuthorizer,
        ContentRepository, ContentScanFailureKind, ContentScanResult, ContentScanner,
        ContentStorageKeyFactory, ContentStorageKeyGenerationResult, ContentUploadClaim,
        ContentUploadClaimOutcome, ContentUploadFingerprint, MatrixRoomId, ObjectStoreFailure,
        ObjectStoreFailureKind, ObjectStoreResult, ObjectWriteReceipt, OpenedContentObject,
        PortFuture, PrivateContentObjectStore, ReclaimableContentQuery,
    },
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    ids::{ContentId, ContentUploadRequestId, PrincipalId},
    time::UtcMillis,
};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[tokio::test]
async fn 重复上传声明返回同一内容而冲突声明被拒绝() {
    let repository = Arc::new(MemoryContentRepository::default());
    let service = begin_service(Arc::clone(&repository));
    let request = upload_request(b"hello", ContentEncryptionMode::ServerSide);

    let first = service.begin(request.clone()).await.expect("首次声明成功");
    let second = service
        .begin(request.clone())
        .await
        .expect("相同声明幂等返回");
    let first_id = outcome_content(&first).id();
    assert!(matches!(first, BeginContentUploadOutcome::Created { .. }));
    assert!(matches!(second, BeginContentUploadOutcome::Existing { .. }));
    assert_eq!(outcome_content(&second).id(), first_id);
    assert_eq!(repository.content_count(), 1);

    let conflicting = BeginContentUploadRequest {
        media_type: ContentMediaType::new("application/json").expect("媒体类型有效"),
        ..request
    };
    let failure = service
        .begin(conflicting)
        .await
        .expect_err("同一幂等键不能改写声明");
    assert!(matches!(
        failure,
        BeginContentUploadFailure::Repository(error)
            if error.kind() == RepositoryErrorKind::Conflict
    ));
}

#[tokio::test]
async fn 非房间成员在生成对象键和写入仓储前被拒绝() {
    let repository = Arc::new(MemoryContentRepository::default());
    let service = begin_service_with_authorizer(
        Arc::clone(&repository),
        Arc::new(FixedAuthorizer(ContentAuthorizationDecision::Denied)),
    );

    let failure = service
        .begin(upload_request(
            b"unauthorized",
            ContentEncryptionMode::ClientE2ee,
        ))
        .await
        .expect_err("非成员不能建立上传会话");

    assert!(matches!(failure, BeginContentUploadFailure::Denied));
    assert_eq!(repository.content_count(), 0);
}

#[tokio::test]
async fn 分块写入经摘要和扫描验证后才激活且重复完成不重写对象() {
    let bytes = b"verified body".to_vec();
    let repository = Arc::new(MemoryContentRepository::default());
    let object_store = Arc::new(MemoryObjectStore::default());
    let scanner = Arc::new(FakeScanner::new(ContentScanState::Clean));
    let begin = begin_service(Arc::clone(&repository));
    let content_id = outcome_content(
        &begin
            .begin(upload_request(&bytes, ContentEncryptionMode::ServerSide))
            .await
            .expect("声明成功"),
    )
    .id();
    let service = complete_service(
        Arc::clone(&repository),
        Arc::clone(&object_store),
        Arc::clone(&scanner),
    );

    let outcome = service
        .complete(CompleteContentUploadRequest {
            principal_id: principal_id(),
            content_id,
            body: chunks(vec![bytes[..4].to_vec(), bytes[4..].to_vec()]),
        })
        .await
        .expect("完成上传");
    assert!(matches!(
        outcome,
        CompleteContentUploadOutcome::Activated(_)
    ));
    assert_eq!(object_store.put_calls(), 1);
    assert_eq!(scanner.calls(), 1);
    assert_eq!(
        repository.content(content_id).lifecycle_state(),
        ContentLifecycleState::Active
    );

    let repeated = service
        .complete(CompleteContentUploadRequest {
            principal_id: principal_id(),
            content_id,
            body: chunks(Vec::new()),
        })
        .await
        .expect("重复完成直接返回现有对象");
    assert!(matches!(
        repeated,
        CompleteContentUploadOutcome::AlreadyActive(_)
    ));
    assert_eq!(object_store.put_calls(), 1);
}

#[tokio::test]
async fn 对象存储返回错误摘要时元数据成为孤儿且对象被删除() {
    let bytes = b"declared bytes".to_vec();
    let repository = Arc::new(MemoryContentRepository::default());
    let object_store = Arc::new(MemoryObjectStore::corrupting_receipt());
    let scanner = Arc::new(FakeScanner::new(ContentScanState::Clean));
    let content_id = outcome_content(
        &begin_service(Arc::clone(&repository))
            .begin(upload_request(&bytes, ContentEncryptionMode::ServerSide))
            .await
            .expect("声明成功"),
    )
    .id();

    let failure = complete_service(Arc::clone(&repository), Arc::clone(&object_store), scanner)
        .complete(CompleteContentUploadRequest {
            principal_id: principal_id(),
            content_id,
            body: chunks(vec![bytes]),
        })
        .await
        .expect_err("摘要不一致必须失败");

    assert!(matches!(
        failure,
        CompleteContentUploadFailure::IntegrityMismatch { .. }
    ));
    assert_eq!(object_store.delete_calls(), 1);
    assert_eq!(
        repository.content(content_id).lifecycle_state(),
        ContentLifecycleState::Orphaned
    );
}

#[tokio::test]
async fn 客户端密文不送入正文扫描器() {
    let bytes = b"ciphertext".to_vec();
    let repository = Arc::new(MemoryContentRepository::default());
    let object_store = Arc::new(MemoryObjectStore::default());
    let scanner = Arc::new(FakeScanner::new(ContentScanState::Rejected));
    let content_id = outcome_content(
        &begin_service(Arc::clone(&repository))
            .begin(upload_request(&bytes, ContentEncryptionMode::ClientE2ee))
            .await
            .expect("密文声明成功"),
    )
    .id();

    complete_service(repository, object_store, Arc::clone(&scanner))
        .complete(CompleteContentUploadRequest {
            principal_id: principal_id(),
            content_id,
            body: chunks(vec![bytes]),
        })
        .await
        .expect("密文按摘要验证后激活");
    assert_eq!(scanner.calls(), 0);
}

#[tokio::test]
async fn 扫描拒绝会关闭访问并进入可回收孤儿态() {
    let bytes = b"suspicious attachment".to_vec();
    let repository = Arc::new(MemoryContentRepository::default());
    let object_store = Arc::new(MemoryObjectStore::default());
    let scanner = Arc::new(FakeScanner::new(ContentScanState::Rejected));
    let content_id = outcome_content(
        &begin_service(Arc::clone(&repository))
            .begin(upload_request(&bytes, ContentEncryptionMode::ServerSide))
            .await
            .expect("声明成功"),
    )
    .id();

    let failure = complete_service(Arc::clone(&repository), Arc::clone(&object_store), scanner)
        .complete(CompleteContentUploadRequest {
            principal_id: principal_id(),
            content_id,
            body: chunks(vec![bytes]),
        })
        .await
        .expect_err("拒绝结果不能激活");
    assert!(matches!(
        failure,
        CompleteContentUploadFailure::ScanRejected {
            outcome: ContentScanState::Rejected,
            ..
        }
    ));
    assert_eq!(object_store.delete_calls(), 1);
    assert_eq!(
        repository.content(content_id).lifecycle_state(),
        ContentLifecycleState::Orphaned
    );
}

fn begin_service(repository: Arc<MemoryContentRepository>) -> BeginContentUploadService {
    begin_service_with_authorizer(
        repository,
        Arc::new(FixedAuthorizer(ContentAuthorizationDecision::Allowed)),
    )
}

fn begin_service_with_authorizer(
    repository: Arc<MemoryContentRepository>,
    authorizer: Arc<dyn ContentMembershipAuthorizer>,
) -> BeginContentUploadService {
    BeginContentUploadService::new(BeginContentUploadDependencies {
        clock: Arc::new(FixedClock),
        identifiers: Arc::new(RandomContentIdentifiers),
        storage_keys: Arc::new(TestStorageKeys),
        repository,
        authorizer,
    })
}

fn complete_service(
    repository: Arc<MemoryContentRepository>,
    object_store: Arc<MemoryObjectStore>,
    scanner: Arc<FakeScanner>,
) -> CompleteContentUploadService {
    CompleteContentUploadService::new(CompleteContentUploadDependencies {
        clock: Arc::new(FixedClock),
        repository,
        object_store,
        scanner,
    })
}

fn upload_request(
    bytes: &[u8],
    encryption_mode: ContentEncryptionMode,
) -> BeginContentUploadRequest {
    BeginContentUploadRequest {
        request_id: ContentUploadRequestId::from_uuid(Uuid::now_v7()),
        owner_principal_id: principal_id(),
        matrix_room_id: MatrixRoomId::new("!room:example.test").expect("房间 ID 有效"),
        access_mode: ContentAccessMode::RoomMember,
        digest: digest(bytes),
        byte_length: ContentByteLength::new(u64::try_from(bytes.len()).expect("长度可转换"))
            .expect("内容非空"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode,
        expires_at: Some(time(9_000)),
    }
}

fn outcome_content(outcome: &BeginContentUploadOutcome) -> &ContentObject {
    match outcome {
        BeginContentUploadOutcome::Created { content, .. }
        | BeginContentUploadOutcome::Existing { content, .. } => content,
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(0x0198_0000_0000_7000_8000_0000_0000_0001))
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn chunks(values: Vec<Vec<u8>>) -> ContentByteStream {
    Box::pin(stream::iter(values.into_iter().map(Ok)))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        time(1_000)
    }
}

struct FixedAuthorizer(ContentAuthorizationDecision);

impl ContentMembershipAuthorizer for FixedAuthorizer {
    fn authorize<'a>(
        &'a self,
        _request: &'a ContentAuthorizationRequest,
    ) -> PortFuture<'a, ContentAuthorizationResult<ContentAuthorizationDecision>> {
        Box::pin(async move { Ok(self.0) })
    }
}

struct RandomContentIdentifiers;

impl ContentIdentifierFactory for RandomContentIdentifiers {
    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }
}

struct TestStorageKeys;

impl ContentStorageKeyFactory for TestStorageKeys {
    fn generate(
        &self,
        content_id: ContentId,
    ) -> ContentStorageKeyGenerationResult<ContentStorageKey> {
        Ok(
            ContentStorageKey::new(format!("content/{content_id}/opaque-random-suffix"))
                .expect("测试对象键有效"),
        )
    }
}

#[derive(Default)]
struct MemoryContentRepository {
    state: Mutex<RepositoryState>,
}

#[derive(Default)]
struct RepositoryState {
    uploads: HashMap<ContentUploadRequestId, (ContentUploadFingerprint, ContentId)>,
    contents: HashMap<ContentId, ContentObject>,
    policies: HashMap<ContentId, ContentAccessPolicy>,
}

impl MemoryContentRepository {
    fn content_count(&self) -> usize {
        self.state.lock().expect("仓储锁有效").contents.len()
    }

    fn content(&self, content_id: ContentId) -> ContentObject {
        self.state
            .lock()
            .expect("仓储锁有效")
            .contents
            .get(&content_id)
            .expect("内容存在")
            .clone()
    }

    fn mutate_content(
        &self,
        content_id: ContentId,
        operation: impl FnOnce(&mut ContentObject) -> Result<(), agent_room_domain::DomainError>,
    ) -> RepositoryResult<ContentObject> {
        let mut state = self.state.lock().expect("仓储锁有效");
        let content = state.contents.get_mut(&content_id).ok_or_else(|| {
            repository_error("content.test.mutate", RepositoryErrorKind::NotFound)
        })?;
        operation(content).map_err(|_| {
            repository_error("content.test.mutate", RepositoryErrorKind::Constraint)
        })?;
        Ok(content.clone())
    }
}

impl ContentRepository for MemoryContentRepository {
    fn claim_upload<'a>(
        &'a self,
        claim: &'a ContentUploadClaim,
    ) -> PortFuture<'a, RepositoryResult<ContentUploadClaimOutcome>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("仓储锁有效");
            if let Some((fingerprint, content_id)) = state.uploads.get(&claim.request_id) {
                if fingerprint != &claim.fingerprint {
                    return Err(repository_error(
                        "content.test.claim",
                        RepositoryErrorKind::Conflict,
                    ));
                }
                return Ok(ContentUploadClaimOutcome::Existing {
                    content: state.contents.get(content_id).expect("内容存在").clone(),
                    access_policy: state.policies.get(content_id).expect("策略存在").clone(),
                });
            }
            let content_id = claim.content.id();
            state
                .uploads
                .insert(claim.request_id, (claim.fingerprint, content_id));
            state.contents.insert(content_id, claim.content.clone());
            state
                .policies
                .insert(content_id, claim.access_policy.clone());
            Ok(ContentUploadClaimOutcome::Created {
                content: claim.content.clone(),
                access_policy: claim.access_policy.clone(),
            })
        })
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
        content_id: ContentId,
        _activated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        Box::pin(async move { self.mutate_content(content_id, ContentObject::activate) })
    }

    fn record_scan(
        &self,
        content_id: ContentId,
        outcome: ContentScanState,
        _scanned_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        Box::pin(
            async move { self.mutate_content(content_id, |content| content.record_scan(outcome)) },
        )
    }

    fn bind_event<'a>(
        &'a self,
        _binding: &'a ContentEventBinding,
    ) -> PortFuture<'a, RepositoryResult<ContentAccessPolicy>> {
        Box::pin(async {
            Err(repository_error(
                "content.test.bind_event",
                RepositoryErrorKind::Unavailable,
            ))
        })
    }

    fn transition<'a>(
        &'a self,
        transition: &'a ContentLifecycleTransition,
    ) -> PortFuture<'a, RepositoryResult<ContentObject>> {
        Box::pin(async move {
            self.mutate_content(transition.content_id, |content| {
                if content.lifecycle_state() != transition.expected {
                    return Err(agent_room_domain::DomainError::InvalidTransition {
                        entity: "content_object",
                        from: content.lifecycle_state().as_str(),
                        to: transition.target.as_str(),
                    });
                }
                match transition.target {
                    ContentLifecycleState::Active => content.activate(),
                    ContentLifecycleState::Orphaned => content.mark_orphaned(),
                    ContentLifecycleState::Redacted => content.redact(),
                    ContentLifecycleState::Expired => content.expire(transition.changed_at),
                    ContentLifecycleState::Deleted => content.mark_deleted(transition.changed_at),
                    ContentLifecycleState::Uploading => {
                        Err(agent_room_domain::DomainError::InvalidTransition {
                            entity: "content_object",
                            from: content.lifecycle_state().as_str(),
                            to: ContentLifecycleState::Uploading.as_str(),
                        })
                    }
                }
            })
        })
    }

    fn list_reclaimable<'a>(
        &'a self,
        _query: &'a ReclaimableContentQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<ContentObject>>> {
        Box::pin(async { Ok(Vec::new()) })
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

fn repository_error(operation: &'static str, kind: RepositoryErrorKind) -> RepositoryError {
    RepositoryError::new(operation, kind)
}

#[derive(Default)]
struct MemoryObjectStore {
    put_calls: AtomicUsize,
    delete_calls: AtomicUsize,
    corrupt_receipt: bool,
}

impl MemoryObjectStore {
    const fn corrupting_receipt() -> Self {
        Self {
            put_calls: AtomicUsize::new(0),
            delete_calls: AtomicUsize::new(0),
            corrupt_receipt: true,
        }
    }

    fn put_calls(&self) -> usize {
        self.put_calls.load(Ordering::SeqCst)
    }

    fn delete_calls(&self) -> usize {
        self.delete_calls.load(Ordering::SeqCst)
    }
}

impl PrivateContentObjectStore for MemoryObjectStore {
    fn put<'a>(
        &'a self,
        _content: &'a ContentObject,
        mut body: ContentByteStream,
    ) -> PortFuture<'a, ObjectStoreResult<ObjectWriteReceipt>> {
        Box::pin(async move {
            self.put_calls.fetch_add(1, Ordering::SeqCst);
            let mut hasher = Sha256::new();
            let mut byte_length = 0_u64;
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(|_| {
                    ObjectStoreFailure::new("content.test.put", ObjectStoreFailureKind::Rejected)
                })?;
                byte_length = byte_length
                    .checked_add(u64::try_from(chunk.len()).map_err(|_| {
                        ObjectStoreFailure::new(
                            "content.test.put",
                            ObjectStoreFailureKind::Rejected,
                        )
                    })?)
                    .ok_or_else(|| {
                        ObjectStoreFailure::new(
                            "content.test.put",
                            ObjectStoreFailureKind::Rejected,
                        )
                    })?;
                hasher.update(chunk);
            }
            let digest = if self.corrupt_receipt {
                Sha256Digest::from_bytes([0xFF; 32])
            } else {
                Sha256Digest::from_bytes(hasher.finalize().into())
            };
            Ok(ObjectWriteReceipt {
                digest,
                byte_length: ContentByteLength::new(byte_length).map_err(|_| {
                    ObjectStoreFailure::new("content.test.put", ObjectStoreFailureKind::Rejected)
                })?,
            })
        })
    }

    fn open<'a>(
        &'a self,
        _content: &'a ContentObject,
    ) -> PortFuture<'a, ObjectStoreResult<OpenedContentObject>> {
        Box::pin(async {
            Err(ObjectStoreFailure::new(
                "content.test.open",
                ObjectStoreFailureKind::NotFound,
            ))
        })
    }

    fn delete<'a>(&'a self, _content: &'a ContentObject) -> PortFuture<'a, ObjectStoreResult<()>> {
        Box::pin(async move {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

struct FakeScanner {
    outcome: ContentScanState,
    calls: AtomicUsize,
}

impl FakeScanner {
    const fn new(outcome: ContentScanState) -> Self {
        Self {
            outcome,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ContentScanner for FakeScanner {
    fn scan<'a>(
        &'a self,
        _content: &'a ContentObject,
    ) -> PortFuture<'a, ContentScanResult<ContentScanState>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.outcome == ContentScanState::Pending {
                return Err(agent_room_application::ports::ContentScanFailure::new(
                    "content.test.scan",
                    ContentScanFailureKind::InvalidResponse,
                ));
            }
            Ok(self.outcome)
        })
    }
}
