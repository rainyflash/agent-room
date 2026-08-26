use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        AutomationConsumptionOutcome, AutomationConsumptionRequest, AutomationGrantRepository,
        AutomationGrantRevocationOutcome, AutomationScopeAuthority,
        AutomationScopeAuthorityRequest, AutomationSendAuthorityRequest, MatrixRoomId,
    },
};
use agent_room_domain::{
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, DeviceId, MessageSubmissionId, PrincipalId,
        RoomCatalogId,
    },
    policy::{
        AutomationAudience, AutomationGrant, AutomationGrantAttempt, AutomationGrantDenial,
        AutomationGrantFields, AutomationGrantLimits, AutomationGrantScope, AutomationGrantStatus,
        AutomationMessageKind, AutomationMessageKinds, AutomationRiskScanOutcome,
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

struct AutomationFixture {
    principal: PrincipalId,
    device: DeviceId,
    agent: AgentId,
    instance: AgentInstanceId,
    catalog: RoomCatalogId,
    matrix_room: MatrixRoomId,
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 自动发言作用域每次读取当前产品权限() {
    let database = TestDatabase::connect().await;
    let fixture = seed_automation_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());

    let create_request = AutomationScopeAuthorityRequest {
        principal_id: fixture.principal,
        agent_id: fixture.agent,
        agent_instance_id: Some(fixture.instance),
        room_catalog_id: fixture.catalog,
    };
    assert!(
        AutomationScopeAuthority::may_create(&repositories, &create_request)
            .await
            .expect("有效所有权与实例应允许签发")
    );

    let send_request = AutomationSendAuthorityRequest {
        principal_id: fixture.principal,
        device_id: fixture.device,
        agent_id: fixture.agent,
        agent_instance_id: fixture.instance,
        room_catalog_id: fixture.catalog,
        matrix_room_id: fixture.matrix_room.clone(),
    };
    let authority = AutomationScopeAuthority::inspect_send(&repositories, &send_request)
        .await
        .expect("当前产品权限读取应成功")
        .expect("在线实例应有发送权威");
    assert!(authority.contains_unknown_recipients);

    sqlx::query(
        "UPDATE agent_room.agent_instance SET status = 'offline', lease_expires_at = NULL WHERE id = $1",
    )
    .bind(fixture.instance.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("测试应能撤销在线资格");
    assert!(
        AutomationScopeAuthority::inspect_send(&repositories, &send_request)
            .await
            .expect("权限变化读取应成功")
            .is_none(),
        "实例离线后必须立即失败关闭"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 自动发言消费在真实数据库中原子限流_幂等_耗尽并撤销() {
    let database = TestDatabase::connect().await;
    let fixture = seed_automation_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let now = database_now(&database.runtime).await;

    let rate_grant = verify_atomic_rate_and_replay(&repositories, &fixture, now).await;
    verify_lifecycle_limits(&database.runtime, &repositories, &fixture, &rate_grant, now).await;

    database.close().await;
}

async fn verify_atomic_rate_and_replay(
    repositories: &PostgresRepositories,
    fixture: &AutomationFixture,
    now: UtcMillis,
) -> AutomationGrant {
    let rate_grant = grant(fixture, now, 1, None, 60_000);
    AutomationGrantRepository::create(repositories, &rate_grant)
        .await
        .expect("频率授权应持久化");
    let first = consumption(fixture, rate_grant.id(), now, submission_id());
    let second = consumption(fixture, rate_grant.id(), now, submission_id());
    let (first_outcome, second_outcome) = tokio::join!(
        AutomationGrantRepository::consume(repositories, &first),
        AutomationGrantRepository::consume(repositories, &second),
    );
    let outcomes = [
        first_outcome.expect("首个事务应完成"),
        second_outcome.expect("次个事务应完成"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                AutomationConsumptionOutcome::Consumed { reused: false, .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                AutomationConsumptionOutcome::Denied(AutomationGrantDenial::RateLimitExceeded)
            ))
            .count(),
        1,
        "授权行锁后的用量快照必须阻止并发超发"
    );

    let winning_request = if matches!(outcomes[0], AutomationConsumptionOutcome::Consumed { .. }) {
        &first
    } else {
        &second
    };
    let replay = AutomationGrantRepository::consume(repositories, winning_request)
        .await
        .expect("安全重试应成功");
    assert!(matches!(
        replay,
        AutomationConsumptionOutcome::Consumed { reused: true, .. }
    ));
    let mut tampered = winning_request.clone();
    tampered.attempt.message_kind = AutomationMessageKind::Reply;
    let conflict = AutomationGrantRepository::consume(repositories, &tampered)
        .await
        .expect_err("相同提交标识不得改写发送意图");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);

    rate_grant
}

async fn verify_lifecycle_limits(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &AutomationFixture,
    rate_grant: &AutomationGrant,
    now: UtcMillis,
) {
    let revoked_at = time(now.value() + 1_000);
    let revoked = AutomationGrantRepository::revoke(
        repositories,
        fixture.principal,
        rate_grant.id(),
        revoked_at,
    )
    .await
    .expect("撤销应成功");
    assert!(matches!(
        revoked,
        AutomationGrantRevocationOutcome::Revoked(_)
    ));
    let after_revoke = consumption(
        fixture,
        rate_grant.id(),
        time(now.value() + 2_000),
        submission_id(),
    );
    assert!(matches!(
        AutomationGrantRepository::consume(repositories, &after_revoke)
            .await
            .expect("撤销后的拒绝应被记录"),
        AutomationConsumptionOutcome::Denied(AutomationGrantDenial::Revoked)
    ));

    let total_grant = grant(fixture, now, 10, Some(1), 60_000);
    AutomationGrantRepository::create(repositories, &total_grant)
        .await
        .expect("总量授权应持久化");
    let total_request = consumption(fixture, total_grant.id(), now, submission_id());
    let exhausted = AutomationGrantRepository::consume(repositories, &total_request)
        .await
        .expect("最后一个名额应成功消费");
    assert!(matches!(
        exhausted,
        AutomationConsumptionOutcome::Consumed { record, reused: false }
            if record.grant.status() == AutomationGrantStatus::Exhausted
                && record.usage.total_messages == 1
    ));

    let expiring = grant(fixture, now, 10, None, 500);
    AutomationGrantRepository::create(repositories, &expiring)
        .await
        .expect("短期授权应持久化");
    let expired =
        AutomationGrantRepository::find(repositories, expiring.id(), time(now.value() + 500))
            .await
            .expect("到期检查应成功")
            .expect("授权应存在");
    assert_eq!(expired.grant.status(), AutomationGrantStatus::Expired);

    let denial_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.automation_denial WHERE grant_id = $1")
            .bind(rate_grant.id().as_uuid())
            .fetch_one(pool)
            .await
            .expect("拒绝审计应可查询");
    assert_eq!(denial_count, 2, "频率耗尽与撤销必须各留下最小审计记录");
}

fn grant(
    fixture: &AutomationFixture,
    starts_at: UtcMillis,
    per_minute: u16,
    total: Option<u32>,
    lifetime_ms: i64,
) -> AutomationGrant {
    let scope = AutomationGrantScope::new(
        fixture.agent,
        Some(fixture.instance),
        fixture.catalog,
        AutomationMessageKinds::new([
            AutomationMessageKind::RoomMessage,
            AutomationMessageKind::Reply,
        ])
        .expect("消息类别有效"),
        AutomationAudience::AnyRoomMember,
        true,
    )
    .expect("公共大厅自动授权作用域有效");
    let limits = AutomationGrantLimits::new(
        per_minute,
        total,
        starts_at,
        time(starts_at.value() + lifetime_ms),
    )
    .expect("授权限额有效");
    AutomationGrant::issue(AutomationGrantFields {
        id: AutomationGrantId::from_uuid(Uuid::now_v7()),
        grantor_id: fixture.principal,
        scope,
        limits,
        created_at: starts_at,
    })
    .expect("授权有效")
}

fn consumption(
    fixture: &AutomationFixture,
    grant_id: AutomationGrantId,
    now: UtcMillis,
    submission_id: MessageSubmissionId,
) -> AutomationConsumptionRequest {
    AutomationConsumptionRequest {
        grant_id,
        submission_id,
        matrix_room_id: fixture.matrix_room.clone(),
        attempt: AutomationGrantAttempt {
            agent_id: fixture.agent,
            agent_instance_id: Some(fixture.instance),
            room_catalog_id: fixture.catalog,
            message_kind: AutomationMessageKind::RoomMessage,
            contains_unknown_recipients: true,
            risk_scan: AutomationRiskScanOutcome::Passed,
            now,
        },
    }
}

async fn seed_automation_fixture(pool: &PgPool) -> AutomationFixture {
    let binding_id = Uuid::now_v7();
    let room_instance_id = Uuid::now_v7();
    let fixture = AutomationFixture {
        principal: PrincipalId::from_uuid(Uuid::now_v7()),
        device: DeviceId::from_uuid(Uuid::now_v7()),
        agent: AgentId::from_uuid(Uuid::now_v7()),
        instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
        catalog: RoomCatalogId::from_uuid(Uuid::now_v7()),
        matrix_room: MatrixRoomId::new(format!(
            "!automation{}:matrix.test",
            room_instance_id.simple()
        ))
        .expect("Matrix 房间标识有效"),
    };

    seed_principal_and_device(pool, &fixture).await;
    seed_agent_instance(pool, &fixture, binding_id).await;
    seed_public_room(pool, &fixture, room_instance_id).await;
    fixture
}

async fn seed_principal_and_device(pool: &PgPool, fixture: &AutomationFixture) {
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES ($1, 'https://issuer.test', $2, $3, '自动化测试主体',
                     'zh-CN', 'active', statement_timestamp(), statement_timestamp(), 0)",
    )
    .bind(fixture.principal.as_uuid())
    .bind(format!(
        "automation-{}",
        fixture.principal.as_uuid().simple()
    ))
    .bind(format!(
        "@principal-{}:matrix.test",
        fixture.principal.as_uuid().simple()
    ))
    .execute(pool)
    .await
    .expect("主体写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.device (
               id, principal_id, label, platform, public_signing_key,
               matrix_device_id, trust_state, last_seen_at, created_at, verified_at
           ) VALUES ($1, $2, '自动化测试设备', 'windows', $3,
                     'AUTOMATION-DEVICE', 'verified', statement_timestamp(),
                     statement_timestamp(), statement_timestamp())",
    )
    .bind(fixture.device.as_uuid())
    .bind(fixture.principal.as_uuid())
    .bind(signing_key(fixture.device.as_uuid()))
    .execute(pool)
    .await
    .expect("设备写入应成功");
}

async fn seed_agent_instance(pool: &PgPool, fixture: &AutomationFixture, binding_id: Uuid) {
    sqlx::query(
        r"INSERT INTO agent_room.agent (
               id, matrix_user_id, slug, display_name, description, visibility,
               lifecycle_state, created_at, updated_at, version
            ) VALUES ($1, $2, $3, '自动化测试 Agent', '', 'private',
                      'suspended', statement_timestamp(), statement_timestamp(), 0)",
    )
    .bind(fixture.agent.as_uuid())
    .bind(format!(
        "@agent-{}:matrix.test",
        fixture.agent.as_uuid().simple()
    ))
    .bind(format!("agent-{}", fixture.agent.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("Agent 写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
               principal_id, agent_id, role, granted_by, created_at
           ) VALUES ($1, $2, 'owner', $1, clock_timestamp())",
    )
    .bind(fixture.principal.as_uuid())
    .bind(fixture.agent.as_uuid())
    .execute(pool)
    .await
    .expect("Agent 所有权写入应成功");
    sqlx::query("UPDATE agent_room.agent SET lifecycle_state = 'active' WHERE id = $1")
        .bind(fixture.agent.as_uuid())
        .execute(pool)
        .await
        .expect("建立 Owner 后应能激活 Agent");
    sqlx::query(
        r"INSERT INTO agent_room.adapter_binding (
               id, agent_id, adapter_type, capability_version, configuration,
               state, created_at, updated_at
            ) VALUES ($1, $2, 'codex', '1', '{}'::jsonb,
                      'active', statement_timestamp(), statement_timestamp())",
    )
    .bind(binding_id)
    .bind(fixture.agent.as_uuid())
    .execute(pool)
    .await
    .expect("适配器绑定写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance (
               id, agent_id, device_id, adapter_binding_id, public_signing_key,
               matrix_device_id, status, lease_expires_at, last_seen_at, created_at
            ) VALUES ($1, $2, $3, $4, $5, 'AUTOMATION-INSTANCE', 'online',
                      statement_timestamp() + interval '5 minutes', statement_timestamp(), statement_timestamp())",
    )
    .bind(fixture.instance.as_uuid())
    .bind(fixture.agent.as_uuid())
    .bind(fixture.device.as_uuid())
    .bind(binding_id)
    .bind(signing_key(fixture.instance.as_uuid()))
    .execute(pool)
    .await
    .expect("Agent 实例写入应成功");
}

async fn seed_public_room(pool: &PgPool, fixture: &AutomationFixture, room_instance_id: Uuid) {
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
               id, kind, slug, name, description, visibility, status,
               created_at, updated_at
            ) VALUES ($1, 'public_lobby', $2, '自动化测试大厅', '', 'public',
                      'active', statement_timestamp(), statement_timestamp())",
    )
    .bind(fixture.catalog.as_uuid())
    .bind(format!("automation-{}", fixture.catalog.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("房间目录写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.room_instance (
               id, catalog_entry_id, matrix_room_id, soft_capacity, hard_capacity,
               member_count_projection, activity_score, state, created_at, updated_at, version
            ) VALUES ($1, $2, $3, 180, 250, 1, 0, 'active',
                      statement_timestamp(), statement_timestamp(), 0)",
    )
    .bind(room_instance_id)
    .bind(fixture.catalog.as_uuid())
    .bind(fixture.matrix_room.as_str())
    .execute(pool)
    .await
    .expect("房间实例写入应成功");
}

async fn database_now(pool: &PgPool) -> UtcMillis {
    let milliseconds: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(pool)
            .await
            .expect("数据库时间应可读取");
    time(milliseconds)
}

fn time(milliseconds: i64) -> UtcMillis {
    UtcMillis::new(milliseconds).expect("测试时间有效")
}

fn submission_id() -> MessageSubmissionId {
    MessageSubmissionId::from_uuid(Uuid::now_v7())
}

fn signing_key(id: Uuid) -> Vec<u8> {
    id.as_bytes().iter().copied().cycle().take(32).collect()
}

fn required_url(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少真实数据库测试配置 {name}"))
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .min_connections(0)
        .max_connections(8)
        .connect(url)
        .await
        .expect("真实 PostgreSQL 必须可连接")
}
