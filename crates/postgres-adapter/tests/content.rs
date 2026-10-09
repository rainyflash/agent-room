use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        ContentAccessMode, ContentAccessPolicy, ContentEventBinding, ContentLifecycleTransition,
        ContentRepository, ContentUploadClaim, ContentUploadClaimOutcome, ContentUploadFingerprint,
        MatrixEventId, MatrixRoomId, ReclaimableContentQuery, RoomRetentionLookup,
    },
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    ids::{ContentId, ContentUploadRequestId, PrincipalId},
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use tokio::task::JoinSet;
use uuid::Uuid;

/// 已经存着的正文补设到期时间的迁移；测试在事务里重跑它，看完就回滚，别的测试看不到。
const RETENTION_BACKFILL: &str =
    include_str!("../../../infra/migrations/202610090003_content_retention_backfill.sql");
const DAY: i64 = 24 * 60 * 60 * 1_000;

struct TestDatabase {
    migration: PgPool,
    runtime: PgPool,
}

impl TestDatabase {
    async fn connect() -> Self {
        let migration = connect_pool(&required_url("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL")).await;
        run_migrations(&migration).await.expect("迁移必须成功");
        let runtime = connect_pool(&required_url("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL")).await;
        Self { migration, runtime }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.migration.close().await;
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 并发重复上传声明只创建一个内容对象() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime).await;
    let claim = upload_claim(owner, ContentUploadRequestId::from_uuid(Uuid::now_v7()), 7);
    let expected_content_id = claim.content.id();
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let mut tasks = JoinSet::new();

    for _ in 0..2 {
        let repositories = repositories.clone();
        let claim = claim.clone();
        tasks.spawn(async move { ContentRepository::claim_upload(&repositories, &claim).await });
    }

    let mut created = 0;
    let mut existing = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("并发任务不能崩溃").expect("声明应成功") {
            ContentUploadClaimOutcome::Created { content, .. } => {
                created += 1;
                assert_eq!(content.id(), expected_content_id);
            }
            ContentUploadClaimOutcome::Existing { content, .. } => {
                existing += 1;
                assert_eq!(content.id(), expected_content_id);
            }
        }
    }
    assert_eq!(created, 1);
    assert_eq!(existing, 1);
    assert_eq!(content_count(&database.runtime, owner).await, 1);
    assert_eq!(upload_request_count(&database.runtime, owner).await, 1);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 同一幂等键不能改写声明或产生孤立对象() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime).await;
    let request_id = ContentUploadRequestId::from_uuid(Uuid::now_v7());
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let original = upload_claim(owner, request_id, 11);
    ContentRepository::claim_upload(&repositories, &original)
        .await
        .expect("首次声明成功");

    let conflicting = upload_claim(owner, request_id, 12);
    let failure = ContentRepository::claim_upload(&repositories, &conflicting)
        .await
        .expect_err("冲突声明必须拒绝");
    assert_eq!(failure.kind(), RepositoryErrorKind::Conflict);
    assert_eq!(content_count(&database.runtime, owner).await, 1);
    assert_eq!(upload_request_count(&database.runtime, owner).await, 1);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 扫描激活事件绑定与孤儿删除按单向状态持久化() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let claim = upload_claim(owner, ContentUploadRequestId::from_uuid(Uuid::now_v7()), 21);
    let content_id = claim.content.id();
    ContentRepository::claim_upload(&repositories, &claim)
        .await
        .expect("声明成功");

    let scanned = ContentRepository::record_scan(
        &repositories,
        content_id,
        ContentScanState::Clean,
        time(2_000),
    )
    .await
    .expect("扫描结果持久化");
    assert_eq!(scanned.scan_state(), ContentScanState::Clean);
    let active = ContentRepository::activate(&repositories, content_id, time(3_000))
        .await
        .expect("激活成功");
    assert_eq!(active.lifecycle_state(), ContentLifecycleState::Active);

    let event_id =
        MatrixEventId::new(format!("$event-{content_id}:matrix.test")).expect("事件 ID 有效");
    let policy = ContentRepository::bind_event(
        &repositories,
        &ContentEventBinding {
            content_id,
            matrix_room_id: claim.access_policy.matrix_room_id().clone(),
            matrix_event_id: event_id.clone(),
            bound_at: time(4_000),
        },
    )
    .await
    .expect("事件绑定成功");
    assert_eq!(policy.matrix_event_id(), Some(&event_id));

    let orphaned = ContentRepository::transition(
        &repositories,
        &ContentLifecycleTransition {
            content_id,
            expected: ContentLifecycleState::Active,
            target: ContentLifecycleState::Orphaned,
            changed_at: time(5_000),
        },
    )
    .await
    .expect("事件失败后进入孤儿态");
    assert_eq!(orphaned.lifecycle_state(), ContentLifecycleState::Orphaned);
    let deleted = ContentRepository::mark_deleted(&repositories, content_id, time(6_000))
        .await
        .expect("对象删除后进入终态");
    assert_eq!(deleted.lifecycle_state(), ContentLifecycleState::Deleted);
    assert_eq!(deleted.deleted_at(), Some(time(6_000)));

    let revival = ContentRepository::activate(&repositories, content_id, time(7_000))
        .await
        .expect_err("删除终态不能复活");
    assert_eq!(revival.kind(), RepositoryErrorKind::Constraint);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 回收查询同时发现卡死上传_到期内容和未绑定事件的活跃对象() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let stale = upload_claim(owner, ContentUploadRequestId::from_uuid(Uuid::now_v7()), 31);
    let expiring = upload_claim(owner, ContentUploadRequestId::from_uuid(Uuid::now_v7()), 32);
    let unbound = upload_claim(owner, ContentUploadRequestId::from_uuid(Uuid::now_v7()), 33);
    ContentRepository::claim_upload(&repositories, &stale)
        .await
        .expect("卡死上传声明成功");
    ContentRepository::claim_upload(&repositories, &expiring)
        .await
        .expect("待到期声明成功");
    ContentRepository::claim_upload(&repositories, &unbound)
        .await
        .expect("未绑定事件声明成功");
    ContentRepository::record_scan(
        &repositories,
        expiring.content.id(),
        ContentScanState::Clean,
        time(2_000),
    )
    .await
    .expect("扫描成功");
    ContentRepository::activate(&repositories, expiring.content.id(), time(3_000))
        .await
        .expect("激活成功");
    ContentRepository::record_scan(
        &repositories,
        unbound.content.id(),
        ContentScanState::Clean,
        time(2_000),
    )
    .await
    .expect("扫描成功");
    ContentRepository::activate(&repositories, unbound.content.id(), time(3_000))
        .await
        .expect("未绑定对象激活成功");

    let candidates = ContentRepository::list_reclaimable(
        &repositories,
        &ReclaimableContentQuery {
            now: time(10_000),
            orphaned_before: time(3_500),
            limit: 20,
        },
    )
    .await
    .expect("回收查询成功");
    let ids: Vec<ContentId> = candidates.iter().map(ContentObject::id).collect();
    assert!(ids.contains(&stale.content.id()));
    assert!(ids.contains(&expiring.content.id()));
    assert!(ids.contains(&unbound.content.id()));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 按房间保留期的到期时间存得下_过了保留期才进回收() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let claim = upload_claim_expiring(
        owner,
        ContentUploadRequestId::from_uuid(Uuid::now_v7()),
        41,
        None,
        Some(time(5_000)),
    );
    let content_id = claim.content.id();
    ContentRepository::claim_upload(&repositories, &claim)
        .await
        .expect("声明成功");
    ContentRepository::record_scan(
        &repositories,
        content_id,
        ContentScanState::Clean,
        time(2_000),
    )
    .await
    .expect("扫描成功");
    ContentRepository::activate(&repositories, content_id, time(3_000))
        .await
        .expect("激活成功");
    // 绑定了消息事件，不会被当成没发出去的对象回收，只剩保留期这一条路。
    ContentRepository::bind_event(
        &repositories,
        &ContentEventBinding {
            content_id,
            matrix_room_id: claim.access_policy.matrix_room_id().clone(),
            matrix_event_id: MatrixEventId::new(format!("$event-{content_id}:matrix.test"))
                .expect("事件 ID 有效"),
            bound_at: time(4_000),
        },
    )
    .await
    .expect("绑定成功");

    let stored = ContentRepository::find_content(&repositories, content_id)
        .await
        .expect("内容可读")
        .expect("内容存在");
    assert_eq!(stored.expires_at(), None);
    assert_eq!(stored.retention_expires_at(), Some(time(5_000)));
    assert!(
        !reclaimable_ids(&repositories, time(4_999))
            .await
            .contains(&content_id)
    );
    assert!(
        reclaimable_ids(&repositories, time(5_000))
            .await
            .contains(&content_id)
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 房间保留期按聊天房间查到目录上的设置() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let week = seed_archived_lobby(&database.migration, Some(7)).await;
    let unset = seed_archived_lobby(&database.migration, None).await;
    let unknown = MatrixRoomId::new(format!("!unknown-{}:matrix.test", Uuid::now_v7().simple()))
        .expect("房间 ID 有效");

    for (room, expected) in [(week, Some(7)), (unset, None), (unknown, None)] {
        let days = RoomRetentionLookup::retention_days(&repositories, &room)
            .await
            .expect("保留期可查");
        assert_eq!(days, expected, "{room:?}");
    }

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 补设已经存着的正文到期时间_按房间保留期再多留一天_查不到的按三十天_设过的不动() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let week = seed_archived_lobby(&database.migration, Some(7)).await;
    let weekly = bound_content(&repositories, owner, 51, Some(week), None).await;
    let unlisted = bound_content(&repositories, owner, 52, None, None).await;
    let already = bound_content(&repositories, owner, 53, None, Some(time(5_000))).await;

    let mut transaction = database.migration.begin().await.expect("开事务");
    sqlx::raw_sql(RETENTION_BACKFILL)
        .execute(&mut *transaction)
        .await
        .expect("补设成功");

    // 上传的时间都是 time(1_000)。
    assert_eq!(
        retention_expiry(&mut transaction, weekly).await,
        Some(time(1_000 + 8 * DAY))
    );
    assert_eq!(
        retention_expiry(&mut transaction, unlisted).await,
        Some(time(1_000 + 31 * DAY))
    );
    assert_eq!(
        retention_expiry(&mut transaction, already).await,
        Some(time(5_000)),
        "设过的不动"
    );
    transaction.rollback().await.expect("回滚");

    database.close().await;
}

/// 上传、扫描、激活并绑定到消息事件的正文。给了 `room` 就放在那个房间，不给就放在目录里查不到的房间。
async fn bound_content(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    marker: u8,
    room: Option<MatrixRoomId>,
    retention_expires_at: Option<UtcMillis>,
) -> ContentId {
    let mut claim = upload_claim_expiring(
        owner,
        ContentUploadRequestId::from_uuid(Uuid::now_v7()),
        marker,
        None,
        retention_expires_at,
    );
    let content_id = claim.content.id();
    if let Some(room) = room {
        claim.access_policy =
            ContentAccessPolicy::new(content_id, room, ContentAccessMode::RoomMember, time(1_000));
    }
    ContentRepository::claim_upload(repositories, &claim)
        .await
        .expect("声明成功");
    ContentRepository::record_scan(
        repositories,
        content_id,
        ContentScanState::Clean,
        time(2_000),
    )
    .await
    .expect("扫描成功");
    ContentRepository::activate(repositories, content_id, time(3_000))
        .await
        .expect("激活成功");
    ContentRepository::bind_event(
        repositories,
        &ContentEventBinding {
            content_id,
            matrix_room_id: claim.access_policy.matrix_room_id().clone(),
            matrix_event_id: MatrixEventId::new(format!("$event-{content_id}:matrix.test"))
                .expect("事件 ID 有效"),
            bound_at: time(4_000),
        },
    )
    .await
    .expect("绑定成功");
    content_id
}

async fn retention_expiry(
    transaction: &mut Transaction<'_, Postgres>,
    content_id: ContentId,
) -> Option<UtcMillis> {
    let millis: Option<i64> = sqlx::query_scalar(
        r"SELECT floor(extract(epoch FROM retention_expires_at) * 1000)::bigint
            FROM agent_room.content_object
           WHERE id = $1",
    )
    .bind(content_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .expect("读到期时间");
    millis.map(time)
}

async fn reclaimable_ids(repositories: &PostgresRepositories, now: UtcMillis) -> Vec<ContentId> {
    ContentRepository::list_reclaimable(
        repositories,
        &ReclaimableContentQuery {
            now,
            orphaned_before: time(1_000),
            limit: 200,
        },
    )
    .await
    .expect("回收查询成功")
    .iter()
    .map(ContentObject::id)
    .collect()
}

/// 归档、不公开的大厅：私人房间要连带成员和状态记录，这里只要目录上的保留期；归档了也不会被
/// 别的测试当成可进的公共大厅。
async fn seed_archived_lobby(pool: &PgPool, retention_days: Option<i32>) -> MatrixRoomId {
    let catalog_id = Uuid::now_v7();
    let room = MatrixRoomId::new(format!("!retention-{}:matrix.test", catalog_id.simple()))
        .expect("房间 ID 有效");
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
               id, kind, name, visibility, retention_days, status, created_at, updated_at
           ) VALUES ($1, 'public_lobby', '保留期测试大厅', 'unlisted', $2, 'archived',
                     statement_timestamp(), statement_timestamp())",
    )
    .bind(catalog_id)
    .bind(retention_days)
    .execute(pool)
    .await
    .expect("房间目录写入成功");
    sqlx::query(
        r"INSERT INTO agent_room.room_instance (
               id, catalog_entry_id, matrix_room_id, state, created_at, updated_at
           ) VALUES ($1, $2, $3, 'archived', statement_timestamp(), statement_timestamp())",
    )
    .bind(Uuid::now_v7())
    .bind(catalog_id)
    .bind(room.as_str())
    .execute(pool)
    .await
    .expect("房间实例写入成功");
    room
}

fn upload_claim(
    owner: PrincipalId,
    request_id: ContentUploadRequestId,
    marker: u8,
) -> ContentUploadClaim {
    upload_claim_expiring(owner, request_id, marker, Some(time(9_000)), None)
}

fn upload_claim_expiring(
    owner: PrincipalId,
    request_id: ContentUploadRequestId,
    marker: u8,
    expires_at: Option<UtcMillis>,
    retention_expires_at: Option<UtcMillis>,
) -> ContentUploadClaim {
    let content_id = ContentId::from_uuid(Uuid::now_v7());
    let created_at = time(1_000);
    let content = ContentObject::begin_upload(ContentObjectFields {
        id: content_id,
        owner_principal_id: owner,
        storage_key: ContentStorageKey::new(format!(
            "content/{content_id}/opaque-{marker:02x}-suffix"
        ))
        .expect("对象键有效"),
        digest: Sha256Digest::from_bytes([marker; 32]),
        byte_length: ContentByteLength::new(128).expect("长度有效"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode: ContentEncryptionMode::ServerSide,
        scan_state: ContentScanState::Pending,
        lifecycle_state: ContentLifecycleState::Uploading,
        expires_at,
        retention_expires_at,
        created_at,
        deleted_at: None,
    })
    .expect("内容有效");
    ContentUploadClaim {
        request_id,
        fingerprint: ContentUploadFingerprint::from_bytes([marker; 32]),
        content,
        access_policy: ContentAccessPolicy::new(
            content_id,
            MatrixRoomId::new(format!("!room-{content_id}:matrix.test")).expect("房间 ID 有效"),
            ContentAccessMode::RoomMember,
            created_at,
        ),
    }
}

async fn seed_principal(pool: &PgPool) -> PrincipalId {
    let id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES (
               $1, 'https://issuer.content.test', $2, $3, '内容测试主体',
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

async fn content_count(pool: &PgPool, owner: PrincipalId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.content_object WHERE owner_principal_id = $1",
    )
    .bind(owner.as_uuid())
    .fetch_one(pool)
    .await
    .expect("内容计数可读")
}

async fn upload_request_count(pool: &PgPool, owner: PrincipalId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.content_upload_request WHERE owner_principal_id = $1",
    )
    .bind(owner.as_uuid())
    .fetch_one(pool)
    .await
    .expect("上传请求计数可读")
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(8)
        .connect(url)
        .await
        .expect("测试数据库必须可连接")
}

fn required_url(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少测试环境变量 {name}"))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
