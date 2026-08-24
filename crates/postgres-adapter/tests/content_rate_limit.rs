use std::env;

use agent_room_application::ports::{
    ContentDownloadAttempt, ContentDownloadLimiter, ContentRateLimitDecision,
    ContentRateLimitFailureKind, MatrixRoomId,
};
use agent_room_domain::{
    content::{ContentByteLength, MAX_CONTENT_BYTES},
    ids::{ContentId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{
    ContentDownloadLimitPolicy, PostgresContentDownloadLimiter, run_migrations,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::task::JoinSet;
use uuid::Uuid;

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
async fn 多副本并发下载严格共享同一配额() {
    let database = TestDatabase::connect().await;
    let principal_id = seed_principal(&database.runtime).await;
    let limiter = limiter(database.runtime.clone(), 5);
    let attempt = attempt(principal_id, 10_000);
    let mut tasks = JoinSet::new();

    for _ in 0..16 {
        let limiter = limiter.clone();
        let attempt = attempt.clone();
        tasks.spawn(async move { ContentDownloadLimiter::check(&limiter, &attempt).await });
    }

    let mut allowed = 0;
    let mut rejected = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("并发任务不能崩溃").expect("限流存储可用") {
            ContentRateLimitDecision::Allowed => allowed += 1,
            ContentRateLimitDecision::RetryAt(retry_at) => {
                rejected += 1;
                assert_eq!(retry_at, time(70_000));
            }
        }
    }
    assert_eq!(allowed, 5);
    assert_eq!(rejected, 11);

    let (request_count, byte_count): (i32, i64) = sqlx::query_as(
        "SELECT request_count, byte_count FROM agent_room.content_download_window WHERE principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("配额窗口可读");
    assert_eq!(request_count, 5);
    assert_eq!(byte_count, 5 * 1_024);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 到达窗口边界后原子开启新窗口() {
    let database = TestDatabase::connect().await;
    let principal_id = seed_principal(&database.runtime).await;
    let limiter = limiter(database.runtime.clone(), 1);

    assert_eq!(
        ContentDownloadLimiter::check(&limiter, &attempt(principal_id, 1_000))
            .await
            .expect("首次下载可判定"),
        ContentRateLimitDecision::Allowed
    );
    assert_eq!(
        ContentDownloadLimiter::check(&limiter, &attempt(principal_id, 60_999))
            .await
            .expect("窗口内下载可判定"),
        ContentRateLimitDecision::RetryAt(time(61_000))
    );
    assert_eq!(
        ContentDownloadLimiter::check(&limiter, &attempt(principal_id, 61_000))
            .await
            .expect("边界下载可判定"),
        ContentRateLimitDecision::Allowed
    );

    let started_at_ms: i64 = sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM window_started_at) * 1000)::bigint FROM agent_room.content_download_window WHERE principal_id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("新窗口可读");
    assert_eq!(started_at_ms, 61_000);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 不存在的主体不会绕过外键创建预算() {
    let database = TestDatabase::connect().await;
    let limiter = limiter(database.runtime.clone(), 1);
    let missing = PrincipalId::from_uuid(Uuid::now_v7());
    let failure = ContentDownloadLimiter::check(&limiter, &attempt(missing, 1_000))
        .await
        .expect_err("未知主体必须失败");
    assert_eq!(failure.kind(), ContentRateLimitFailureKind::Unavailable);

    database.close().await;
}

fn limiter(pool: PgPool, max_downloads: u32) -> PostgresContentDownloadLimiter {
    let policy = ContentDownloadLimitPolicy::new(
        DurationMillis::new(60_000).expect("窗口有效"),
        max_downloads,
        MAX_CONTENT_BYTES,
    )
    .expect("测试策略有效");
    PostgresContentDownloadLimiter::new(pool, policy)
}

fn attempt(principal_id: PrincipalId, attempted_at: i64) -> ContentDownloadAttempt {
    ContentDownloadAttempt {
        principal_id,
        content_id: ContentId::from_uuid(Uuid::now_v7()),
        matrix_room_id: MatrixRoomId::new("!content-rate-limit:matrix.test").expect("房间 ID 有效"),
        byte_length: ContentByteLength::new(1_024).expect("字节长度有效"),
        attempted_at: time(attempted_at),
    }
}

async fn seed_principal(pool: &PgPool) -> PrincipalId {
    let id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
              id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
              locale, status, created_at, updated_at, version
          ) VALUES (
              $1, 'https://issuer.rate-limit.test', $2, $3, '下载限流测试主体',
              'zh-CN', 'active', to_timestamp(1), to_timestamp(1), 0
          )",
    )
    .bind(id.as_uuid())
    .bind(id.to_string())
    .bind(format!("@rate_limit_{}:matrix.test", id.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("测试主体写入成功");
    id
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(20)
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
