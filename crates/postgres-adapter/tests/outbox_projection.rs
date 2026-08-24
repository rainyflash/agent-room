use std::{env, num::NonZeroU16};

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        ActivityScoreMillis, AgentRegistration, AgentRegistrationTransaction, AgentRepository,
        MatrixMembership, MatrixProjectionBatch, MatrixProjectionEvent, MatrixProjectionEventKind,
        MatrixProjectionRebuild, MatrixProjectionStore, OutboxClaim, OutboxFailure,
        OutboxFailureOutcome, OutboxMessage, OutboxRepository, PrincipalRegistration,
        PrincipalRepository, ProjectionApplyOutcome, ProjectionHealth, ProjectionHealthReport,
        ROOM_PROJECTION_CONSUMER,
    },
};
use agent_room_domain::{
    agents::{Agent, AgentVisibility},
    identity::Principal,
    ids::{AgentId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId},
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use serde_json::{Map, Value};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use uuid::Uuid;

const CONSUMER: &str = ROOM_PROJECTION_CONSUMER;

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
async fn 业务写入与_outbox_原子提交且过期租约可被接管() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = create_principal(&repositories, "Outbox 原子性主体").await;
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let event_id = OutboxEventId::from_uuid(Uuid::now_v7());
    let registration = agent_registration(agent_id, principal_id, "Outbox Agent");
    let event = registration_event(event_id, agent_id, "Outbox Agent");

    AgentRegistrationTransaction::create_with_event(&repositories, &registration, &event)
        .await
        .expect("Agent 与 Outbox 必须原子创建");
    let persisted: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_room.agent WHERE id = $1), \
                (SELECT count(*) FROM agent_room.outbox_event WHERE id = $2)",
    )
    .bind(agent_id.as_uuid())
    .bind(event_id.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("应能核对原子写入");
    assert_eq!(persisted, (1, 1));

    let first_claim = claim("worker-a", 1_700_000_000_100, 1_700_000_000_300);
    let claimed = OutboxRepository::claim(&repositories, &first_claim)
        .await
        .expect("首次领取应成功");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].message().id(), event_id);

    let premature = claim("worker-b", 1_700_000_000_200, 1_700_000_000_400);
    assert!(
        OutboxRepository::claim(&repositories, &premature)
            .await
            .expect("有效租约期间查询应成功")
            .is_empty()
    );

    let takeover = claim("worker-b", 1_700_000_000_301, 1_700_000_000_600);
    let reclaimed = OutboxRepository::claim(&repositories, &takeover)
        .await
        .expect("消费者崩溃后租约必须可接管");
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].worker_name(), "worker-b");
    OutboxRepository::mark_published(&repositories, event_id, "worker-b", time(1_700_000_000_500))
        .await
        .expect("当前租约持有者应能完成事件");

    let second_agent_id = AgentId::from_uuid(Uuid::now_v7());
    let second_registration = agent_registration(second_agent_id, principal_id, "回滚验证 Agent");
    let duplicate_event = registration_event(event_id, second_agent_id, "回滚验证 Agent");
    let error = AgentRegistrationTransaction::create_with_event(
        &repositories,
        &second_registration,
        &duplicate_event,
    )
    .await
    .expect_err("Outbox 主键冲突必须让业务写入一起回滚");
    assert_eq!(error.kind(), RepositoryErrorKind::Conflict);
    let rolled_back: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.agent WHERE id = $1")
            .bind(second_agent_id.as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("应能核对事务回滚");
    assert_eq!(rolled_back, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn outbox_失败会退避并在阈值处进入可观察死信() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let baseline = OutboxRepository::backlog(&repositories, time(1_700_000_100_000))
        .await
        .expect("应能读取基线积压");
    let principal_id = create_principal(&repositories, "Outbox 退避主体").await;
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let event_id = OutboxEventId::from_uuid(Uuid::now_v7());
    let registration = agent_registration(agent_id, principal_id, "退避 Agent");
    let event = registration_event(event_id, agent_id, "退避 Agent");
    AgentRegistrationTransaction::create_with_event(&repositories, &registration, &event)
        .await
        .expect("应创建待发布事件");

    let first_claim = claim("retry-worker-a", 1_700_000_000_100, 1_700_000_000_300);
    OutboxRepository::claim(&repositories, &first_claim)
        .await
        .expect("应领取事件");
    let first_failure = OutboxFailure::new(
        "matrix.unavailable".to_owned(),
        time(1_700_000_000_200),
        time(1_700_000_000_400),
        NonZeroU16::new(2).expect("失败阈值有效"),
    )
    .expect("失败指令有效");
    assert_eq!(
        OutboxRepository::record_failure(
            &repositories,
            event_id,
            "retry-worker-a",
            &first_failure,
        )
        .await
        .expect("首次失败应被记录"),
        OutboxFailureOutcome::RetryScheduled { attempt_count: 1 }
    );
    let scheduled = OutboxRepository::backlog(&repositories, time(1_700_000_000_300))
        .await
        .expect("应能观察退避积压");
    assert_eq!(scheduled.scheduled(), baseline.scheduled() + 1);

    let second_claim = claim("retry-worker-b", 1_700_000_000_401, 1_700_000_000_700);
    OutboxRepository::claim(&repositories, &second_claim)
        .await
        .expect("退避结束后应能再次领取");
    let second_failure = OutboxFailure::new(
        "matrix.unavailable".to_owned(),
        time(1_700_000_000_500),
        time(1_700_000_000_800),
        NonZeroU16::new(2).expect("失败阈值有效"),
    )
    .expect("失败指令有效");
    assert_eq!(
        OutboxRepository::record_failure(
            &repositories,
            event_id,
            "retry-worker-b",
            &second_failure,
        )
        .await
        .expect("第二次失败应进入死信"),
        OutboxFailureOutcome::DeadLettered { attempt_count: 2 }
    );
    let final_backlog = OutboxRepository::backlog(&repositories, time(1_700_000_001_000))
        .await
        .expect("应能观察死信");
    assert_eq!(final_backlog.dead_lettered(), baseline.dead_lettered() + 1);
    assert_eq!(final_backlog.ready(), baseline.ready());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn matrix_投影对崩溃重放重复事件游标回退和全量重建均保持幂等() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = create_principal(&repositories, "投影测试主体").await;
    let first_agent_id = AgentId::from_uuid(Uuid::now_v7());
    let second_agent_id = AgentId::from_uuid(Uuid::now_v7());
    AgentRepository::create(
        &repositories,
        &agent_registration(first_agent_id, principal_id, "投影 Agent A"),
    )
    .await
    .expect("应创建第一个 Agent");
    AgentRepository::create(
        &repositories,
        &agent_registration(second_agent_id, principal_id, "投影 Agent B"),
    )
    .await
    .expect("应创建第二个 Agent");
    let room_id = create_room(&database.runtime).await;

    let activity = verify_initial_projection_and_crash_replay(
        &repositories,
        &database.runtime,
        room_id,
        first_agent_id,
        second_agent_id,
    )
    .await;
    verify_cursor_and_event_guards(
        &repositories,
        &database.runtime,
        room_id,
        first_agent_id,
        activity,
    )
    .await;
    verify_rebuild_and_health(
        &repositories,
        &database.runtime,
        room_id,
        first_agent_id,
        second_agent_id,
    )
    .await;

    database.close().await;
}

async fn verify_initial_projection_and_crash_replay(
    repositories: &PostgresRepositories,
    pool: &PgPool,
    room_id: RoomInstanceId,
    first_agent_id: AgentId,
    second_agent_id: AgentId,
) -> MatrixProjectionEvent {
    let joined = membership_event(
        "$membership-a-join",
        1,
        room_id,
        first_agent_id,
        MatrixMembership::Join,
    );
    let activity = activity_event("$activity-a", 2, room_id, 1_500);
    let invited = membership_event(
        "$membership-b-invite",
        3,
        room_id,
        second_agent_id,
        MatrixMembership::Invite,
    );
    let initial_batch = MatrixProjectionBatch::new(
        CONSUMER.to_owned(),
        None,
        "sync-1".to_owned(),
        vec![joined.clone(), activity.clone(), invited],
        time(1_700_000_001_000),
    )
    .expect("初始批次有效");
    assert_eq!(
        MatrixProjectionStore::apply(repositories, &initial_batch)
            .await
            .expect("初始投影应成功"),
        ProjectionApplyOutcome::Applied {
            new_events: 3,
            duplicates: 0,
        }
    );
    assert_room_projection(pool, room_id, 1, 1.5).await;

    assert_eq!(
        MatrixProjectionStore::apply(repositories, &initial_batch)
            .await
            .expect("提交成功但调用方崩溃后的整批重放应幂等"),
        ProjectionApplyOutcome::Replayed { duplicates: 3 }
    );
    assert_room_projection(pool, room_id, 1, 1.5).await;
    activity
}

async fn verify_cursor_and_event_guards(
    repositories: &PostgresRepositories,
    pool: &PgPool,
    room_id: RoomInstanceId,
    first_agent_id: AgentId,
    activity: MatrixProjectionEvent,
) {
    let left = membership_event(
        "$membership-a-leave",
        4,
        room_id,
        first_agent_id,
        MatrixMembership::Leave,
    );
    let stale_batch = MatrixProjectionBatch::new(
        CONSUMER.to_owned(),
        Some("sync-0".to_owned()),
        "sync-stale".to_owned(),
        vec![left.clone()],
        time(1_700_000_002_000),
    )
    .expect("旧游标批次结构有效");
    let error = MatrixProjectionStore::apply(repositories, &stale_batch)
        .await
        .expect_err("旧游标不得覆盖新游标");
    assert_eq!(error.kind(), RepositoryErrorKind::Conflict);
    assert_eq!(
        MatrixProjectionStore::cursor(repositories, CONSUMER)
            .await
            .expect("应能读取游标")
            .expect("游标应存在")
            .sync_token(),
        "sync-1"
    );
    assert_room_projection(pool, room_id, 1, 1.5).await;

    let valid_leave = MatrixProjectionBatch::new(
        CONSUMER.to_owned(),
        Some("sync-1".to_owned()),
        "sync-2".to_owned(),
        vec![left],
        time(1_700_000_002_100),
    )
    .expect("新游标批次有效");
    MatrixProjectionStore::apply(repositories, &valid_leave)
        .await
        .expect("合法游标应推进");
    assert_room_projection(pool, room_id, 0, 1.5).await;

    let duplicate_event_batch = MatrixProjectionBatch::new(
        CONSUMER.to_owned(),
        Some("sync-2".to_owned()),
        "sync-3".to_owned(),
        vec![activity.clone()],
        time(1_700_000_003_000),
    )
    .expect("重复事件批次有效");
    assert_eq!(
        MatrixProjectionStore::apply(repositories, &duplicate_event_batch)
            .await
            .expect("同摘要重复事件应安全跳过"),
        ProjectionApplyOutcome::Applied {
            new_events: 0,
            duplicates: 1,
        }
    );
    assert_room_projection(pool, room_id, 0, 1.5).await;

    let forged_activity = activity_event("$activity-a", 9, room_id, 9_000);
    let forged_batch = MatrixProjectionBatch::new(
        CONSUMER.to_owned(),
        Some("sync-3".to_owned()),
        "sync-4".to_owned(),
        vec![forged_activity],
        time(1_700_000_004_000),
    )
    .expect("伪造批次结构有效");
    let error = MatrixProjectionStore::apply(repositories, &forged_batch)
        .await
        .expect_err("相同事件 ID 的不同摘要必须响亮失败");
    assert_eq!(error.kind(), RepositoryErrorKind::CorruptData);
    assert_eq!(
        MatrixProjectionStore::cursor(repositories, CONSUMER)
            .await
            .expect("应能读取游标")
            .expect("游标应存在")
            .sync_token(),
        "sync-3"
    );
    assert_room_projection(pool, room_id, 0, 1.5).await;
}

async fn verify_rebuild_and_health(
    repositories: &PostgresRepositories,
    pool: &PgPool,
    room_id: RoomInstanceId,
    first_agent_id: AgentId,
    second_agent_id: AgentId,
) {
    let rebuild = MatrixProjectionRebuild::new(
        CONSUMER.to_owned(),
        "sync-rebuilt".to_owned(),
        vec![
            membership_event(
                "$snapshot-a-join",
                10,
                room_id,
                first_agent_id,
                MatrixMembership::Join,
            ),
            membership_event(
                "$snapshot-b-join",
                11,
                room_id,
                second_agent_id,
                MatrixMembership::Join,
            ),
            activity_event("$snapshot-activity", 12, room_id, 2_500),
        ],
        time(1_700_000_005_000),
    )
    .expect("重建快照有效");
    MatrixProjectionStore::rebuild(repositories, &rebuild)
        .await
        .expect("投影应可从快照重建");
    assert_room_projection(pool, room_id, 2, 2.5).await;
    MatrixProjectionStore::rebuild(repositories, &rebuild)
        .await
        .expect("重复重建结果必须确定");
    assert_room_projection(pool, room_id, 2, 2.5).await;

    let health = ProjectionHealthReport::new(
        CONSUMER.to_owned(),
        ProjectionHealth::Lagging,
        Some("matrix.sync_timeout".to_owned()),
        time(1_700_000_006_000),
    )
    .expect("健康报告有效");
    MatrixProjectionStore::report_health(repositories, &health)
        .await
        .expect("应持久化投影延迟状态");
    let lookup = MatrixProjectionStore::membership(repositories, CONSUMER, room_id, first_agent_id)
        .await
        .expect("应能读取成员投影")
        .expect("游标存在时应返回投影查询上下文");
    assert_eq!(lookup.membership(), Some(MatrixMembership::Join));
    assert_eq!(lookup.health(), ProjectionHealth::Lagging);
}

fn registration_event(
    event_id: OutboxEventId,
    agent_id: AgentId,
    display_name: &str,
) -> OutboxMessage {
    let mut payload = Map::new();
    payload.insert("version".to_owned(), Value::from(1));
    payload.insert("display_name".to_owned(), Value::from(display_name));
    OutboxMessage::new(
        event_id,
        "agent".to_owned(),
        agent_id.as_uuid(),
        "agent.registered.v1".to_owned(),
        payload,
        time(1_700_000_000_000),
    )
    .expect("注册事件有效")
}

fn claim(worker_name: &str, claimed_at: i64, lease_expires_at: i64) -> OutboxClaim {
    OutboxClaim::new(
        worker_name.to_owned(),
        NonZeroU16::new(10).expect("批大小有效"),
        time(claimed_at),
        time(lease_expires_at),
    )
    .expect("领取请求有效")
}

async fn create_principal(repositories: &PostgresRepositories, display_name: &str) -> PrincipalId {
    let id = PrincipalId::from_uuid(Uuid::now_v7());
    let registration = PrincipalRegistration {
        principal: Principal::new(id),
        oidc_issuer: "https://issuer.test".to_owned(),
        oidc_subject: format!("subject-{id}"),
        matrix_user_id: format!("@principal-{id}:matrix.agent-room.localhost"),
        display_name: display_name.to_owned(),
        avatar_content_id: None,
        locale: "zh-CN".to_owned(),
        registered_at: time(1_700_000_000_000),
    };
    PrincipalRepository::create(repositories, &registration)
        .await
        .expect("测试主体应创建成功");
    id
}

fn agent_registration(
    agent_id: AgentId,
    principal_id: PrincipalId,
    display_name: &str,
) -> AgentRegistration {
    AgentRegistration {
        agent: Agent::register(agent_id),
        owner_id: principal_id,
        matrix_user_id: format!("@agent-{agent_id}:matrix.agent-room.localhost"),
        slug: format!("agent-{agent_id}"),
        display_name: display_name.to_owned(),
        description: "真实 PostgreSQL 事件测试".to_owned(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: time(1_700_000_000_000),
    }
}

async fn create_room(pool: &PgPool) -> RoomInstanceId {
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let room_id = RoomInstanceId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
            id, kind, slug, name, description, language, visibility,
            status, created_at, updated_at
        ) VALUES (
            $1, 'public_lobby', $2, '投影测试大厅', '', 'zh-CN', 'public',
            'active', to_timestamp($3::double precision / 1000.0),
            to_timestamp($3::double precision / 1000.0)
        )",
    )
    .bind(catalog_id.as_uuid())
    .bind(format!("projection-{catalog_id}"))
    .bind(1_700_000_000_000_i64)
    .execute(pool)
    .await
    .expect("应创建大厅目录");
    sqlx::query(
        r"INSERT INTO agent_room.room_instance (
            id, catalog_entry_id, matrix_room_id, state, created_at, updated_at
        ) VALUES (
            $1, $2, $3, 'active',
            to_timestamp($4::double precision / 1000.0),
            to_timestamp($4::double precision / 1000.0)
        )",
    )
    .bind(room_id.as_uuid())
    .bind(catalog_id.as_uuid())
    .bind(format!("!room-{room_id}:matrix.agent-room.localhost"))
    .bind(1_700_000_000_000_i64)
    .execute(pool)
    .await
    .expect("应创建大厅实例");
    room_id
}

fn membership_event(
    event_id: &str,
    digest_byte: u8,
    room_instance_id: RoomInstanceId,
    agent_id: AgentId,
    membership: MatrixMembership,
) -> MatrixProjectionEvent {
    MatrixProjectionEvent::new(
        event_id.to_owned(),
        [digest_byte; 32],
        MatrixProjectionEventKind::MembershipChanged {
            room_instance_id,
            agent_id,
            membership,
            power_level: 0,
        },
    )
    .expect("成员事件有效")
}

fn activity_event(
    event_id: &str,
    digest_byte: u8,
    room_instance_id: RoomInstanceId,
    score_millis: u32,
) -> MatrixProjectionEvent {
    MatrixProjectionEvent::new(
        event_id.to_owned(),
        [digest_byte; 32],
        MatrixProjectionEventKind::ActivityObserved {
            room_instance_id,
            score: ActivityScoreMillis::new(score_millis).expect("活动度有效"),
        },
    )
    .expect("活动事件有效")
}

async fn assert_room_projection(
    pool: &PgPool,
    room_id: RoomInstanceId,
    expected_members: i32,
    expected_activity: f64,
) {
    let row = sqlx::query(
        "SELECT member_count_projection, activity_score::double precision AS activity_score \
         FROM agent_room.room_instance WHERE id = $1",
    )
    .bind(room_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能读取房间投影");
    let members: i32 = row
        .try_get("member_count_projection")
        .expect("人数应可解码");
    let activity: f64 = row.try_get("activity_score").expect("活动度应可解码");
    assert_eq!(members, expected_members);
    assert!((activity - expected_activity).abs() < f64::EPSILON);
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
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
