use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use agent_room_application::{
    content::{
        ContentReadTicketLifetime, IssueContentReadTicketDependencies,
        IssueContentReadTicketRequest, IssueContentReadTicketService, OpenContentDependencies,
        OpenContentFailure, OpenContentRequest, OpenContentService,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, ContentAccessMode, ContentAccessPolicy, ContentAuthorizationDecision,
        ContentAuthorizationRequest, ContentAuthorizationResult, ContentByteStream,
        ContentDownloadAttempt, ContentDownloadLimiter, ContentEventBinding,
        ContentLifecycleTransition, ContentMembershipAuthorizer, ContentRateLimitDecision,
        ContentRateLimitResult, ContentReadTicket, ContentReadTicketClaims, ContentReadTicketCodec,
        ContentRepository, ContentStreamFailureKind, ContentTicketFailure,
        ContentTicketFailureKind, ContentTicketResult, ContentUploadClaim,
        ContentUploadClaimOutcome, MatrixEventId, MatrixRoomId, ObjectStoreFailure,
        ObjectStoreFailureKind, ObjectStoreResult, ObjectWriteReceipt, OpenedContentObject,
        PortFuture, PrivateContentObjectStore, ReclaimableContentQuery,
    },
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    ids::{ContentId, PrincipalId},
    time::UtcMillis,
};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[tokio::test]
async fn 票据只为已绑定事件的当前房间成员签发() {
    let fixture = ReadFixture::new(b"original");
    let service = fixture.issue_service();

    let issued = service
        .issue(IssueContentReadTicketRequest {
            principal_id: fixture.principal_id,
            actor_agent_id: None,
            content_id: fixture.content.id(),
        })
        .await
        .expect("当前成员可取得短期票据");

    assert_eq!(issued.expires_at, time(121_000));
    let claims = fixture.codec.issued_claims();
    assert_eq!(claims.content_id, fixture.content.id());
    assert_eq!(claims.matrix_event_id, fixture.event_id);
    assert_eq!(claims.expires_at, issued.expires_at);
}

#[tokio::test]
async fn 票据签发后权限变化会在真正读取前重新拒绝() {
    let fixture = ReadFixture::new(b"original");
    let ticket = fixture.issue_ticket().await;
    fixture.authorizer.set_allowed(false);

    let failure = fixture
        .open_service()
        .open(OpenContentRequest {
            principal_id: fixture.principal_id,
            content_id: fixture.content.id(),
            ticket,
        })
        .await
        .expect_err("离开房间后旧票据必须立即失效");

    assert_eq!(failure, OpenContentFailure::Denied);
    assert_eq!(fixture.object_store.open_calls(), 0);
}

#[tokio::test]
async fn 对象正文被篡改时流末尾明确报告摘要不一致() {
    let fixture = ReadFixture::new(b"original");
    fixture.object_store.replace_body(b"tampered");
    let ticket = fixture.issue_ticket().await;
    let mut opened = fixture
        .open_service()
        .open(OpenContentRequest {
            principal_id: fixture.principal_id,
            content_id: fixture.content.id(),
            ticket,
        })
        .await
        .expect("对象元数据仍匹配时可建立校验流");

    let first = opened.body.next().await.expect("先读取正文分块");
    assert_eq!(first.expect("首个分块传输成功"), b"tampered");
    let terminal = opened.body.next().await.expect("流末尾必须产生校验结果");
    assert_eq!(
        terminal.expect_err("篡改正文必须失败").kind(),
        ContentStreamFailureKind::IntegrityMismatch
    );
    assert!(opened.body.next().await.is_none());
}

#[tokio::test]
async fn 对象元数据不一致时不会向调用方暴露任何正文() {
    let fixture = ReadFixture::new(b"original");
    fixture.object_store.corrupt_metadata();
    let ticket = fixture.issue_ticket().await;

    let failure = fixture
        .open_service()
        .open(OpenContentRequest {
            principal_id: fixture.principal_id,
            content_id: fixture.content.id(),
            ticket,
        })
        .await
        .expect_err("损坏元数据必须在返回流之前失败");
    assert_eq!(failure, OpenContentFailure::ObjectMetadataMismatch);
}

#[tokio::test]
async fn 过期票据在查询内容和对象存储之前被拒绝() {
    let fixture = ReadFixture::new(b"original");
    fixture.codec.set_verify_failure(ContentTicketFailure::new(
        "content.test.verify",
        ContentTicketFailureKind::Expired,
    ));

    let failure = fixture
        .open_service()
        .open(OpenContentRequest {
            principal_id: fixture.principal_id,
            content_id: fixture.content.id(),
            ticket: ContentReadTicket::new("expired-ticket").expect("票据格式有效"),
        })
        .await
        .expect_err("过期票据必须失败");
    assert!(matches!(
        failure,
        OpenContentFailure::Ticket(error) if error.kind() == ContentTicketFailureKind::Expired
    ));
    assert_eq!(fixture.repository.find_calls(), 0);
    assert_eq!(fixture.object_store.open_calls(), 0);
}

#[tokio::test]
async fn 路径内容与票据内容不一致时不会查询仓储() {
    let fixture = ReadFixture::new(b"original");
    let ticket = fixture.issue_ticket().await;
    let repository_calls_before_open = fixture.repository.find_calls();

    let failure = fixture
        .open_service()
        .open(OpenContentRequest {
            principal_id: fixture.principal_id,
            content_id: ContentId::from_uuid(Uuid::now_v7()),
            ticket,
        })
        .await
        .expect_err("路径内容必须与票据声明一致");

    assert_eq!(failure, OpenContentFailure::StaleTicket);
    assert_eq!(
        fixture.repository.find_calls(),
        repository_calls_before_open
    );
    assert_eq!(fixture.object_store.open_calls(), 0);
}

struct ReadFixture {
    principal_id: PrincipalId,
    content: ContentObject,
    event_id: MatrixEventId,
    repository: Arc<StaticContentRepository>,
    authorizer: Arc<ToggleAuthorizer>,
    codec: Arc<RecordingTicketCodec>,
    limiter: Arc<AllowingLimiter>,
    object_store: Arc<StaticObjectStore>,
}

impl ReadFixture {
    fn new(payload: &[u8]) -> Self {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let content = active_content(principal_id, payload);
        let room_id = MatrixRoomId::new("!content:example.test").expect("房间 ID 有效");
        let event_id = MatrixEventId::new("$content-event").expect("事件 ID 有效");
        let policy = ContentAccessPolicy::restore(
            content.id(),
            room_id.clone(),
            Some(event_id.clone()),
            ContentAccessMode::RoomMember,
            time(1_000),
            None,
        )
        .expect("访问策略有效");
        let claims = ContentReadTicketClaims {
            principal_id,
            actor_agent_id: None,
            content_id: content.id(),
            matrix_room_id: room_id,
            matrix_event_id: event_id.clone(),
            digest: content.digest(),
            byte_length: content.byte_length(),
            media_type: content.media_type().clone(),
            issued_at: time(1_000),
            expires_at: time(121_000),
        };
        Self {
            principal_id,
            repository: Arc::new(StaticContentRepository::new(content.clone(), policy)),
            authorizer: Arc::new(ToggleAuthorizer::new(true)),
            codec: Arc::new(RecordingTicketCodec::new(claims)),
            limiter: Arc::new(AllowingLimiter),
            object_store: Arc::new(StaticObjectStore::new(content.clone(), payload)),
            content,
            event_id,
        }
    }

    fn issue_service(&self) -> IssueContentReadTicketService {
        IssueContentReadTicketService::new(IssueContentReadTicketDependencies {
            clock: Arc::new(FixedClock),
            repository: self.repository.clone(),
            authorizer: self.authorizer.clone(),
            ticket_codec: self.codec.clone(),
            lifetime: ContentReadTicketLifetime::new(120_000).expect("票据寿命有效"),
        })
    }

    fn open_service(&self) -> OpenContentService {
        OpenContentService::new(OpenContentDependencies {
            clock: Arc::new(FixedClock),
            repository: self.repository.clone(),
            authorizer: self.authorizer.clone(),
            ticket_codec: self.codec.clone(),
            limiter: self.limiter.clone(),
            object_store: self.object_store.clone(),
        })
    }

    async fn issue_ticket(&self) -> ContentReadTicket {
        self.issue_service()
            .issue(IssueContentReadTicketRequest {
                principal_id: self.principal_id,
                actor_agent_id: None,
                content_id: self.content.id(),
            })
            .await
            .expect("签发成功")
            .ticket
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        time(1_000)
    }
}

struct StaticContentRepository {
    content: ContentObject,
    policy: ContentAccessPolicy,
    find_calls: AtomicUsize,
}

impl StaticContentRepository {
    const fn new(content: ContentObject, policy: ContentAccessPolicy) -> Self {
        Self {
            content,
            policy,
            find_calls: AtomicUsize::new(0),
        }
    }

    fn find_calls(&self) -> usize {
        self.find_calls.load(Ordering::SeqCst)
    }
}

impl ContentRepository for StaticContentRepository {
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
        self.find_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok((self.content.id() == content_id).then(|| self.content.clone())) })
    }

    fn find_access_policy(
        &self,
        content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentAccessPolicy>>> {
        Box::pin(async move {
            Ok((self.policy.content_id() == content_id).then(|| self.policy.clone()))
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
        _binding: &'a ContentEventBinding,
    ) -> PortFuture<'a, RepositoryResult<ContentAccessPolicy>> {
        unsupported_repository()
    }

    fn transition<'a>(
        &'a self,
        _transition: &'a ContentLifecycleTransition,
    ) -> PortFuture<'a, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }

    fn list_reclaimable<'a>(
        &'a self,
        _query: &'a ReclaimableContentQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<ContentObject>>> {
        unsupported_repository()
    }

    fn mark_deleted(
        &self,
        _content_id: ContentId,
        _deleted_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        unsupported_repository()
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

struct ToggleAuthorizer {
    allowed: AtomicBool,
}

impl ToggleAuthorizer {
    const fn new(allowed: bool) -> Self {
        Self {
            allowed: AtomicBool::new(allowed),
        }
    }

    fn set_allowed(&self, allowed: bool) {
        self.allowed.store(allowed, Ordering::SeqCst);
    }
}

impl ContentMembershipAuthorizer for ToggleAuthorizer {
    fn authorize<'a>(
        &'a self,
        _request: &'a ContentAuthorizationRequest,
    ) -> PortFuture<'a, ContentAuthorizationResult<ContentAuthorizationDecision>> {
        Box::pin(async move {
            Ok(if self.allowed.load(Ordering::SeqCst) {
                ContentAuthorizationDecision::Allowed
            } else {
                ContentAuthorizationDecision::Denied
            })
        })
    }
}

struct RecordingTicketCodec {
    claims: ContentReadTicketClaims,
    issued: Mutex<Option<ContentReadTicketClaims>>,
    verify_failure: Mutex<Option<ContentTicketFailure>>,
}

impl RecordingTicketCodec {
    const fn new(claims: ContentReadTicketClaims) -> Self {
        Self {
            claims,
            issued: Mutex::new(None),
            verify_failure: Mutex::new(None),
        }
    }

    fn issued_claims(&self) -> ContentReadTicketClaims {
        self.issued
            .lock()
            .expect("票据记录锁有效")
            .clone()
            .expect("已经签发票据")
    }

    fn set_verify_failure(&self, failure: ContentTicketFailure) {
        *self.verify_failure.lock().expect("票据失败锁有效") = Some(failure);
    }
}

impl ContentReadTicketCodec for RecordingTicketCodec {
    fn issue<'a>(
        &'a self,
        claims: &'a ContentReadTicketClaims,
    ) -> PortFuture<'a, ContentTicketResult<ContentReadTicket>> {
        Box::pin(async move {
            *self.issued.lock().expect("票据记录锁有效") = Some(claims.clone());
            ContentReadTicket::new("signed-short-lived-ticket").map_err(|_| {
                ContentTicketFailure::new("content.test.issue", ContentTicketFailureKind::Invalid)
            })
        })
    }

    fn verify<'a>(
        &'a self,
        _ticket: &'a ContentReadTicket,
        expected_principal_id: PrincipalId,
        _now: UtcMillis,
    ) -> PortFuture<'a, ContentTicketResult<ContentReadTicketClaims>> {
        Box::pin(async move {
            if let Some(failure) = self.verify_failure.lock().expect("票据失败锁有效").clone()
            {
                return Err(failure);
            }
            if self.claims.principal_id != expected_principal_id {
                return Err(ContentTicketFailure::new(
                    "content.test.verify",
                    ContentTicketFailureKind::AudienceMismatch,
                ));
            }
            Ok(self.claims.clone())
        })
    }
}

struct AllowingLimiter;

impl ContentDownloadLimiter for AllowingLimiter {
    fn check<'a>(
        &'a self,
        _attempt: &'a ContentDownloadAttempt,
    ) -> PortFuture<'a, ContentRateLimitResult<ContentRateLimitDecision>> {
        Box::pin(async { Ok(ContentRateLimitDecision::Allowed) })
    }
}

struct StaticObjectStore {
    content: ContentObject,
    body: Mutex<Vec<u8>>,
    metadata_valid: AtomicBool,
    open_calls: AtomicUsize,
}

impl StaticObjectStore {
    fn new(content: ContentObject, body: &[u8]) -> Self {
        Self {
            content,
            body: Mutex::new(body.to_vec()),
            metadata_valid: AtomicBool::new(true),
            open_calls: AtomicUsize::new(0),
        }
    }

    fn replace_body(&self, body: &[u8]) {
        *self.body.lock().expect("对象正文锁有效") = body.to_vec();
    }

    fn corrupt_metadata(&self) {
        self.metadata_valid.store(false, Ordering::SeqCst);
    }

    fn open_calls(&self) -> usize {
        self.open_calls.load(Ordering::SeqCst)
    }
}

impl PrivateContentObjectStore for StaticObjectStore {
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
        Box::pin(async move {
            self.open_calls.fetch_add(1, Ordering::SeqCst);
            let metadata_valid = self.metadata_valid.load(Ordering::SeqCst);
            let bytes = self.body.lock().expect("对象正文锁有效").clone();
            Ok(OpenedContentObject {
                reported_digest: Some(if metadata_valid {
                    self.content.digest()
                } else {
                    Sha256Digest::from_bytes([0xff; 32])
                }),
                reported_byte_length: Some(self.content.byte_length()),
                body: Box::pin(stream::once(async { Ok(bytes) })),
            })
        })
    }

    fn delete<'a>(&'a self, _content: &'a ContentObject) -> PortFuture<'a, ObjectStoreResult<()>> {
        unsupported_object_store()
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

fn active_content(owner_principal_id: PrincipalId, payload: &[u8]) -> ContentObject {
    let content_id = ContentId::from_uuid(Uuid::now_v7());
    let mut content = ContentObject::begin_upload(ContentObjectFields {
        id: content_id,
        owner_principal_id,
        storage_key: ContentStorageKey::new(format!("content/{content_id}/opaque-random-suffix"))
            .expect("对象键有效"),
        digest: Sha256Digest::from_bytes(Sha256::digest(payload).into()),
        byte_length: ContentByteLength::new(u64::try_from(payload.len()).expect("长度可转换"))
            .expect("内容长度有效"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode: ContentEncryptionMode::ServerSide,
        scan_state: ContentScanState::Clean,
        lifecycle_state: ContentLifecycleState::Uploading,
        expires_at: Some(time(500_000)),
        created_at: time(500),
        deleted_at: None,
    })
    .expect("测试内容有效");
    content.activate().expect("内容可激活");
    content
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
