use std::env;

use agent_room_application::persistence::RepositoryErrorKind;
use agent_room_application::ports::{
    DeviceProofNonceStore, DeviceRefreshOutcome, DeviceRefreshReplay, DeviceRefreshReplaySalt,
    DeviceRegistrationTransaction, DeviceRepository, DeviceRevocationOutcome,
    DeviceRevocationTransaction, DeviceSecurityEvent, DeviceSessionRegistration,
    DeviceSessionStore, DeviceTokenReplacement, PrincipalRegistration, SecretDigest,
};
use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey, DeviceTokenFamily},
    identity::Principal,
    ids::{
        DeviceAccessTokenId, DeviceId, DeviceRefreshAttemptId, DeviceRefreshTokenId,
        DeviceTokenFamilyId, OutboxEventId, PrincipalId,
    },
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, postgres::PgPoolOptions};
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
async fn 设备撤销会原子失效_token_并写入传播事件() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let registration = registration(11, 12);
    let stored = DeviceRegistrationTransaction::register(
        &repositories,
        &registration.principal,
        &registration.device,
        &registration.session,
    )
    .await
    .expect("设备与 Token 应原子写入");
    let replay = DeviceRegistrationTransaction::register(
        &repositories,
        &registration.principal,
        &registration.device,
        &registration.session,
    )
    .await
    .expect_err("同一 OIDC 设备授权断言不得重复注册");
    assert_eq!(replay.kind(), RepositoryErrorKind::Conflict);

    assert!(
        DeviceSessionStore::find_active_access(
            &repositories,
            &registration.session.access_token_digest,
            test_time(1_000),
        )
        .await
        .expect("访问 Token 可查询")
        .is_some()
    );
    assert!(
        DeviceProofNonceStore::consume(
            &repositories,
            stored.device.id(),
            &SecretDigest::from_array([99; 32]),
            test_time(1_000),
            test_time(61_000),
        )
        .await
        .expect("首次 nonce 可消费")
    );
    assert!(
        !DeviceProofNonceStore::consume(
            &repositories,
            stored.device.id(),
            &SecretDigest::from_array([99; 32]),
            test_time(2_000),
            test_time(62_000),
        )
        .await
        .expect("重复 nonce 返回冲突结果")
    );

    let event = security_event(2_000);
    let outcome = DeviceRevocationTransaction::revoke(
        &repositories,
        stored.account.principal.id(),
        stored.device.id(),
        event,
    )
    .await
    .expect("设备撤销成功");
    assert_eq!(outcome, DeviceRevocationOutcome::Revoked(Vec::new()));
    assert!(
        DeviceSessionStore::find_active_access(
            &repositories,
            &registration.session.access_token_digest,
            test_time(3_000),
        )
        .await
        .expect("撤销后可查询")
        .is_none()
    );
    let listed = DeviceRepository::list_for_principal(&repositories, stored.account.principal.id())
        .await
        .expect("设备列表可查询");
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].accepts_authenticated_requests());
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.outbox_event WHERE id = $1 AND event_type = $2",
    )
    .bind(event.id.as_uuid())
    .bind("device.revoked.v1")
    .fetch_one(&database.runtime)
    .await
    .expect("可检查撤销传播事件");
    assert_eq!(event_count, 1);

    delete_device_events(&database.runtime, stored.device.id()).await;
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 并发刷新只能一次成功且旧令牌重用会提交泄露撤销() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let registration = registration(21, 22);
    let stored = DeviceRegistrationTransaction::register(
        &repositories,
        &registration.principal,
        &registration.device,
        &registration.session,
    )
    .await
    .expect("设备与 Token 应原子写入");
    let first_replacement = replacement(31, 32, 1_000);
    let second_replacement = replacement(41, 42, 1_000);

    let (first, second) = tokio::join!(
        DeviceSessionStore::rotate_refresh(
            &repositories,
            &registration.session.refresh_token_digest,
            &first_replacement,
            security_event(2_000),
        ),
        DeviceSessionStore::rotate_refresh(
            &repositories,
            &registration.session.refresh_token_digest,
            &second_replacement,
            security_event(3_000),
        )
    );
    let outcomes = [
        first.expect("首个并发刷新应返回"),
        second.expect("第二个并发刷新应返回"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, DeviceRefreshOutcome::Rotated { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, DeviceRefreshOutcome::ReuseDetected { .. }))
            .count(),
        1
    );

    let state: (String, String) = sqlx::query_as(
        r"SELECT device.trust_state, family.state
           FROM agent_room.device AS device
           JOIN agent_room.device_token_family AS family ON family.device_id = device.id
           WHERE device.id = $1",
    )
    .bind(stored.device.id().as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("可读取泄露状态");
    assert_eq!(state, ("revoked".to_owned(), "compromised".to_owned()));
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.outbox_event WHERE aggregate_id = $1 AND event_type = $2",
    )
    .bind(stored.device.id().as_uuid())
    .bind("device.compromised.v1")
    .fetch_one(&database.runtime)
    .await
    .expect("可检查泄露传播事件");
    assert_eq!(event_count, 1);

    delete_device_events(&database.runtime, stored.device.id()).await;
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 同一尝试号的并发重试重放原轮换且不提交泄露撤销() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let registration = registration(51, 52);
    let stored = register(&repositories, &registration).await;
    let attempt = DeviceRefreshAttemptId::from_uuid(Uuid::now_v7());
    let original = attempted_replacement(53, 54, 1_000, attempt, 7);
    let rotated = rotate(
        &repositories,
        &registration.session.refresh_token_digest,
        &original,
    )
    .await;
    assert!(matches!(rotated, DeviceRefreshOutcome::Rotated { .. }));

    // 重试时服务端会为请求抽取新盐与新令牌，重放必须忽略它们，交回原轮换的盐和摘要。
    let first_request = attempted_replacement(55, 56, 2_000, attempt, 8);
    let second_request = attempted_replacement(57, 58, 2_000, attempt, 9);
    let (first_retry, second_retry) = tokio::join!(
        rotate(
            &repositories,
            &registration.session.refresh_token_digest,
            &first_request
        ),
        rotate(
            &repositories,
            &registration.session.refresh_token_digest,
            &second_request
        ),
    );

    for retried in [first_retry, second_retry] {
        assert_replays(&retried, &original, stored.device.id());
    }
    assert_eq!(
        device_state(&database.runtime, stored.device.id()).await,
        ("verified".to_owned(), "active".to_owned())
    );
    let next = rotate(
        &repositories,
        &original.refresh_token_digest,
        &attempted_replacement(59, 60, 3_000, new_attempt(), 10),
    )
    .await;
    assert!(
        matches!(next, DeviceRefreshOutcome::Rotated { .. }),
        "重放交回的新令牌仍是当前令牌，可以继续轮换"
    );

    delete_device_events(&database.runtime, stored.device.id()).await;
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 新令牌再次轮换后清空派生盐且迟到的旧尝试按重用处理() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let registration = registration(61, 62);
    let stored = register(&repositories, &registration).await;
    let attempt = new_attempt();
    let original = attempted_replacement(63, 64, 1_000, attempt, 12);
    rotate(
        &repositories,
        &registration.session.refresh_token_digest,
        &original,
    )
    .await;
    rotate(
        &repositories,
        &original.refresh_token_digest,
        &attempted_replacement(65, 66, 2_000, new_attempt(), 13),
    )
    .await;

    let retained_salt: Option<Vec<u8>> = sqlx::query_scalar(
        r"SELECT attempt.replay_salt
           FROM agent_room.device_refresh_attempt AS attempt
           JOIN agent_room.device_refresh_token AS refresh
             ON refresh.id = attempt.refresh_token_id
           WHERE refresh.secret_digest = $1",
    )
    .bind(
        registration
            .session
            .refresh_token_digest
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(&database.runtime)
    .await
    .expect("可读取上一轮尝试记录");
    assert_eq!(retained_salt, None, "新令牌轮换后不应再保留可重放的派生盐");

    let late_retry = rotate(
        &repositories,
        &registration.session.refresh_token_digest,
        &attempted_replacement(67, 68, 3_000, attempt, 14),
    )
    .await;
    assert!(matches!(
        late_retry,
        DeviceRefreshOutcome::ReuseDetected { .. }
    ));
    assert_eq!(
        device_state(&database.runtime, stored.device.id()).await,
        ("revoked".to_owned(), "compromised".to_owned())
    );

    delete_device_events(&database.runtime, stored.device.id()).await;
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 换一个尝试号重放已轮换的令牌仍提交泄露撤销() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let registration = registration(71, 72);
    let stored = register(&repositories, &registration).await;
    rotate(
        &repositories,
        &registration.session.refresh_token_digest,
        &attempted_replacement(73, 74, 1_000, new_attempt(), 1),
    )
    .await;

    let reuse = rotate(
        &repositories,
        &registration.session.refresh_token_digest,
        &attempted_replacement(75, 76, 2_000, new_attempt(), 2),
    )
    .await;

    assert!(matches!(reuse, DeviceRefreshOutcome::ReuseDetected { .. }));
    assert_eq!(
        device_state(&database.runtime, stored.device.id()).await,
        ("revoked".to_owned(), "compromised".to_owned())
    );

    delete_device_events(&database.runtime, stored.device.id()).await;
    database.close().await;
}

async fn register(
    repositories: &PostgresRepositories,
    registration: &RegistrationFixture,
) -> agent_room_application::ports::StoredDeviceSession {
    DeviceRegistrationTransaction::register(
        repositories,
        &registration.principal,
        &registration.device,
        &registration.session,
    )
    .await
    .expect("设备与 Token 应原子写入")
}

async fn rotate(
    repositories: &PostgresRepositories,
    refresh_token_digest: &SecretDigest,
    replacement: &DeviceTokenReplacement,
) -> DeviceRefreshOutcome {
    DeviceSessionStore::rotate_refresh(
        repositories,
        refresh_token_digest,
        replacement,
        security_event(replacement.issued_at.value() - test_time(0).value()),
    )
    .await
    .expect("刷新轮换应返回确定结果")
}

fn assert_replays(
    outcome: &DeviceRefreshOutcome,
    original: &DeviceTokenReplacement,
    device_id: DeviceId,
) {
    let DeviceRefreshOutcome::Replayed {
        session,
        salt,
        access_token_digest,
        refresh_token_digest,
        ..
    } = outcome
    else {
        panic!("同一尝试号的重试必须重放原轮换，实际为 {outcome:?}");
    };
    let original_replay = original.replay.as_ref().expect("原轮换带尝试号");
    assert_eq!(salt, &original_replay.salt);
    assert_eq!(access_token_digest, &original.access_token_digest);
    assert_eq!(refresh_token_digest, &original.refresh_token_digest);
    assert_eq!(
        session.access_token_expires_at,
        original.access_token_expires_at
    );
    assert_eq!(session.device.id(), device_id);
}

fn new_attempt() -> DeviceRefreshAttemptId {
    DeviceRefreshAttemptId::from_uuid(Uuid::now_v7())
}

struct RegistrationFixture {
    principal: PrincipalRegistration,
    device: Device,
    session: DeviceSessionRegistration,
}

fn registration(access_digest: u8, refresh_digest: u8) -> RegistrationFixture {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let principal = PrincipalRegistration {
        principal: Principal::new(principal_id),
        oidc_issuer: "https://issuer.example/realms/agent-room".to_owned(),
        oidc_subject: Uuid::now_v7().to_string(),
        matrix_user_id: format!("@user-{principal_id}:matrix.example"),
        display_name: "设备测试用户".to_owned(),
        avatar_content_id: None,
        locale: "zh-CN".to_owned(),
        registered_at: test_time(0),
    };
    let mut device = Device::register(
        DeviceId::from_uuid(Uuid::now_v7()),
        principal_id,
        "测试工作站".to_owned(),
        DevicePlatform::Windows,
        DevicePublicSigningKey::new(Uuid::now_v7().as_bytes().repeat(2)).expect("32 字节公钥有效"),
        test_time(0),
    )
    .expect("设备有效");
    device.verify().expect("设备已完成持有证明");
    let family = DeviceTokenFamily::new(
        DeviceTokenFamilyId::from_uuid(Uuid::now_v7()),
        device.id(),
        test_time(0),
        test_time(30 * 24 * 60 * 60 * 1_000),
    )
    .expect("Token 族有效");
    let session = DeviceSessionRegistration {
        authorization_token_digest: SecretDigest::from_array([access_digest.wrapping_add(100); 32]),
        authorization_receipt_expires_at: test_time(10 * 60 * 1_000),
        family,
        access_token_id: DeviceAccessTokenId::from_uuid(Uuid::now_v7()),
        access_token_digest: SecretDigest::from_array([access_digest; 32]),
        access_token_expires_at: test_time(5 * 60 * 1_000),
        refresh_token_id: DeviceRefreshTokenId::from_uuid(Uuid::now_v7()),
        refresh_token_digest: SecretDigest::from_array([refresh_digest; 32]),
        issued_at: test_time(0),
    };
    RegistrationFixture {
        principal,
        device,
        session,
    }
}

fn replacement(access_digest: u8, refresh_digest: u8, offset: i64) -> DeviceTokenReplacement {
    DeviceTokenReplacement {
        access_token_id: DeviceAccessTokenId::from_uuid(Uuid::now_v7()),
        access_token_digest: SecretDigest::from_array([access_digest; 32]),
        access_token_expires_at: test_time(offset + 5 * 60 * 1_000),
        refresh_token_id: DeviceRefreshTokenId::from_uuid(Uuid::now_v7()),
        refresh_token_digest: SecretDigest::from_array([refresh_digest; 32]),
        issued_at: test_time(offset),
        replay: None,
    }
}

fn attempted_replacement(
    access_digest: u8,
    refresh_digest: u8,
    offset: i64,
    attempt_id: DeviceRefreshAttemptId,
    salt: u8,
) -> DeviceTokenReplacement {
    DeviceTokenReplacement {
        replay: Some(DeviceRefreshReplay {
            attempt_id,
            salt: DeviceRefreshReplaySalt::from_array([salt; 32]),
        }),
        ..replacement(access_digest, refresh_digest, offset)
    }
}

async fn device_state(pool: &PgPool, device_id: DeviceId) -> (String, String) {
    sqlx::query_as(
        r"SELECT device.trust_state, family.state
           FROM agent_room.device AS device
           JOIN agent_room.device_token_family AS family ON family.device_id = device.id
           WHERE device.id = $1",
    )
    .bind(device_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("可读取设备与 Token 族状态")
}

fn security_event(offset: i64) -> DeviceSecurityEvent {
    DeviceSecurityEvent {
        id: OutboxEventId::from_uuid(Uuid::now_v7()),
        occurred_at: test_time(offset),
    }
}

fn test_time(offset: i64) -> UtcMillis {
    UtcMillis::new(1_700_000_000_000 + offset).expect("测试时间有效")
}

fn required_url(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少真实数据库测试配置 {name}"))
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .min_connections(0)
        .max_connections(5)
        .connect(url)
        .await
        .expect("真实 PostgreSQL 必须可连接")
}

async fn delete_device_events(pool: &PgPool, device_id: DeviceId) {
    sqlx::query("DELETE FROM agent_room.outbox_event WHERE aggregate_id = $1")
        .bind(device_id.as_uuid())
        .execute(pool)
        .await
        .expect("测试必须清理自己创建的传播事件");
}
