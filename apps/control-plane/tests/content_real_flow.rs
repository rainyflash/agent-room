use std::{env, net::SocketAddr, sync::Arc, time::Duration};

use agent_room_application::{
    content::{
        BeginContentUploadDependencies, BeginContentUploadOutcome, BeginContentUploadRequest,
        BeginContentUploadService, CleanupContentDependencies, CleanupContentPolicy,
        CleanupContentService, CompleteContentUploadDependencies, CompleteContentUploadFailure,
        CompleteContentUploadOutcome, CompleteContentUploadRequest, CompleteContentUploadService,
        ContentIdentifierFactory,
    },
    ports::{
        Clock, ContentAccessMode, ContentAuthorizationDecision, ContentAuthorizationRequest,
        ContentAuthorizationResult, ContentByteStream, ContentMembershipAuthorizer,
        ContentRepository, PortFuture, PrivateContentObjectStore, SecretValue,
    },
};
use agent_room_content_adapter::{
    ClamAvContentScanner, ClamAvScannerConfig, S3ContentStoreConfig, S3PrivateContentObjectStore,
    SecureContentStorageKeyFactory,
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        Sha256Digest,
    },
    ids::{ContentId, ContentUploadRequestId, PrincipalId},
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const CLEAN_PAYLOAD: &[u8] = b"Agent Room verified content pipeline";
const REJECTED_PAYLOAD: &[u8] = b"AgentRoom-ClamAV-Integration-Marker-5f6e2a91";

#[tokio::test]
#[ignore = "需要由 tools/content.py 提供隔离 PostgreSQL、SeaweedFS 与 ClamAV"]
async fn 真实内容管线验证幂等扫描补偿与事件失败回收() {
    let pipeline = ContentPipeline::connect().await;
    let clean_content_id = verify_clean_upload(&pipeline).await;
    let rejected_content_id = verify_rejected_upload(&pipeline).await;
    verify_event_failure_cleanup(&pipeline, [clean_content_id, rejected_content_id]).await;
    pipeline.close().await;
}

async fn verify_clean_upload(pipeline: &ContentPipeline) -> ContentId {
    let request_id = ContentUploadRequestId::from_uuid(Uuid::now_v7());
    let request = upload_request(pipeline.owner, request_id, CLEAN_PAYLOAD);
    let first = pipeline
        .begin
        .begin(request.clone())
        .await
        .expect("首次声明成功");
    let repeated = pipeline.begin.begin(request).await.expect("重复声明成功");
    let content_id = outcome_content_id(&first);
    assert!(matches!(first, BeginContentUploadOutcome::Created { .. }));
    assert!(matches!(
        repeated,
        BeginContentUploadOutcome::Existing { .. }
    ));
    assert_eq!(outcome_content_id(&repeated), content_id);

    let activated = pipeline
        .complete
        .complete(CompleteContentUploadRequest {
            principal_id: pipeline.owner,
            content_id,
            body: body(CLEAN_PAYLOAD),
        })
        .await
        .expect("干净对象通过真实扫描");
    assert!(matches!(
        activated,
        CompleteContentUploadOutcome::Activated(_)
    ));
    let content = pipeline
        .repository
        .find_content(content_id)
        .await
        .expect("仓储查询成功")
        .expect("干净内容存在");
    let opened = pipeline
        .object_store
        .open(&content)
        .await
        .expect("私有对象可读取");
    assert_eq!(collect(opened.body).await, CLEAN_PAYLOAD);
    content_id
}

async fn verify_rejected_upload(pipeline: &ContentPipeline) -> ContentId {
    let rejected = pipeline
        .begin
        .begin(upload_request(
            pipeline.owner,
            ContentUploadRequestId::from_uuid(Uuid::now_v7()),
            REJECTED_PAYLOAD,
        ))
        .await
        .expect("恶意样本声明成功");
    let content_id = outcome_content_id(&rejected);
    let failure = pipeline
        .complete
        .complete(CompleteContentUploadRequest {
            principal_id: pipeline.owner,
            content_id,
            body: body(REJECTED_PAYLOAD),
        })
        .await
        .expect_err("本地恶意标记必须被真实扫描器拒绝");
    assert!(
        matches!(&failure, CompleteContentUploadFailure::ScanRejected { .. }),
        "ClamAV 返回了非预期失败：{failure:?}"
    );
    assert_eq!(
        pipeline
            .repository
            .find_content(content_id)
            .await
            .expect("仓储查询成功")
            .expect("拒绝内容仍保留审计元数据")
            .lifecycle_state(),
        ContentLifecycleState::Orphaned
    );
    content_id
}

async fn verify_event_failure_cleanup(pipeline: &ContentPipeline, content_ids: [ContentId; 2]) {
    let cleanup = CleanupContentService::new(CleanupContentDependencies {
        clock: Arc::new(FixedClock::at(20_000)),
        repository: pipeline.repository.clone(),
        object_store: pipeline.object_store.clone(),
        policy: CleanupContentPolicy::new(1_000, 20).expect("回收策略有效"),
    });
    let cleanup_outcome = cleanup.run().await.expect("回收批次成功");
    assert_eq!(cleanup_outcome.examined, 2);
    assert_eq!(cleanup_outcome.deleted, 2);
    assert!(cleanup_outcome.failures.is_empty());
    for content_id in content_ids {
        let content = pipeline
            .repository
            .find_content(content_id)
            .await
            .expect("仓储查询成功")
            .expect("删除终态保留元数据");
        assert_eq!(content.lifecycle_state(), ContentLifecycleState::Deleted);
        pipeline
            .object_store
            .open(&content)
            .await
            .expect_err("回收后对象不存在");
    }
}

struct ContentPipeline {
    database: TestDatabase,
    owner: PrincipalId,
    repository: Arc<dyn ContentRepository>,
    object_store: Arc<dyn PrivateContentObjectStore>,
    begin: BeginContentUploadService,
    complete: CompleteContentUploadService,
}

impl ContentPipeline {
    async fn connect() -> Self {
        let database = TestDatabase::connect().await;
        let owner = seed_principal(&database.runtime).await;
        let repositories = Arc::new(PostgresRepositories::new(database.runtime.clone()));
        let repository: Arc<dyn ContentRepository> = repositories;
        let object_store = object_store();
        let scanner = Arc::new(ClamAvContentScanner::new(
            scanner_configuration(),
            object_store.clone(),
        ));
        let begin = BeginContentUploadService::new(BeginContentUploadDependencies {
            clock: Arc::new(FixedClock::at(1_000)),
            identifiers: Arc::new(RandomContentIdentifier),
            storage_keys: Arc::new(SecureContentStorageKeyFactory),
            repository: repository.clone(),
            authorizer: Arc::new(AllowRoomMembership),
        });
        let complete = CompleteContentUploadService::new(CompleteContentUploadDependencies {
            clock: Arc::new(FixedClock::at(2_000)),
            repository: repository.clone(),
            object_store: object_store.clone(),
            scanner,
        });
        Self {
            database,
            owner,
            repository,
            object_store,
            begin,
            complete,
        }
    }

    async fn close(self) {
        self.database.close().await;
    }
}

struct TestDatabase {
    migration: PgPool,
    runtime: PgPool,
}

impl TestDatabase {
    async fn connect() -> Self {
        let migration = connect_pool(&required("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL")).await;
        run_migrations(&migration).await.expect("迁移必须成功");
        let runtime = connect_pool(&required("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL")).await;
        Self { migration, runtime }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.migration.close().await;
    }
}

struct FixedClock(UtcMillis);

impl FixedClock {
    fn at(value: i64) -> Self {
        Self(UtcMillis::new(value).expect("测试时间有效"))
    }
}

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        self.0
    }
}

struct RandomContentIdentifier;

impl ContentIdentifierFactory for RandomContentIdentifier {
    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }
}

struct AllowRoomMembership;

impl ContentMembershipAuthorizer for AllowRoomMembership {
    fn authorize<'a>(
        &'a self,
        _request: &'a ContentAuthorizationRequest,
    ) -> PortFuture<'a, ContentAuthorizationResult<ContentAuthorizationDecision>> {
        Box::pin(async { Ok(ContentAuthorizationDecision::Allowed) })
    }
}

fn object_store() -> Arc<dyn PrivateContentObjectStore> {
    let configuration = S3ContentStoreConfig::new(
        required("AGENT_ROOM_TEST_S3_ENDPOINT"),
        required("AGENT_ROOM_TEST_S3_BUCKET"),
        required("AGENT_ROOM_TEST_S3_REGION"),
        SecretValue::new(required("AGENT_ROOM_TEST_S3_ACCESS_KEY")).expect("访问密钥有效"),
        SecretValue::new(required("AGENT_ROOM_TEST_S3_SECRET_KEY")).expect("秘密密钥有效"),
        Duration::from_secs(15),
    )
    .expect("对象存储配置有效");
    Arc::new(S3PrivateContentObjectStore::new(&configuration))
}

fn scanner_configuration() -> ClamAvScannerConfig {
    let address = required("AGENT_ROOM_TEST_CLAMAV_ADDRESS")
        .parse::<SocketAddr>()
        .expect("ClamAV 地址有效");
    ClamAvScannerConfig::new(address, Duration::from_secs(2), Duration::from_secs(30))
        .expect("扫描配置有效")
}

fn upload_request(
    owner_principal_id: PrincipalId,
    request_id: ContentUploadRequestId,
    payload: &[u8],
) -> BeginContentUploadRequest {
    BeginContentUploadRequest {
        request_id,
        owner_principal_id,
        matrix_room_id: agent_room_application::ports::MatrixRoomId::new(
            "!content-pipeline:matrix.test",
        )
        .expect("房间 ID 有效"),
        access_mode: ContentAccessMode::RoomMember,
        digest: Sha256Digest::from_bytes(Sha256::digest(payload).into()),
        byte_length: ContentByteLength::new(u64::try_from(payload.len()).expect("正文长度可转换"))
            .expect("正文非空"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode: ContentEncryptionMode::ServerSide,
        expires_at: None,
    }
}

fn outcome_content_id(outcome: &BeginContentUploadOutcome) -> ContentId {
    match outcome {
        BeginContentUploadOutcome::Created { content, .. }
        | BeginContentUploadOutcome::Existing { content, .. } => content.id(),
    }
}

fn body(payload: &[u8]) -> ContentByteStream {
    let midpoint = payload.len() / 2;
    Box::pin(stream::iter([
        Ok(payload[..midpoint].to_vec()),
        Ok(payload[midpoint..].to_vec()),
    ]))
}

async fn collect(mut body: ContentByteStream) -> Vec<u8> {
    let mut collected = Vec::new();
    while let Some(chunk) = body.next().await {
        collected.extend(chunk.expect("对象流读取成功"));
    }
    collected
}

async fn seed_principal(pool: &PgPool) -> PrincipalId {
    let id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES (
               $1, 'https://issuer.content-pipeline.test', $2, $3, '内容管线测试主体',
               'zh-CN', 'active', to_timestamp(1), to_timestamp(1), 0
           )",
    )
    .bind(id.as_uuid())
    .bind(id.to_string())
    .bind(format!("@content_{}:matrix.test", id.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("测试主体写入成功");
    id
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("测试数据库必须可连接")
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少测试环境变量 {name}"))
}
