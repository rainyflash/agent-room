use std::{borrow::Cow, env};

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{AgentRegistration, AgentRepository, PrincipalRegistration, PrincipalRepository},
};
use agent_room_domain::{
    agents::{Agent, AgentVisibility},
    identity::Principal,
    ids::{AgentId, PrincipalId},
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const EXPECTED_TABLES: [&str; 27] = [
    "adapter_binding",
    "agent",
    "agent_card_snapshot",
    "agent_instance",
    "agent_ownership",
    "audit_event",
    "automation_grant",
    "content_access_policy",
    "content_object",
    "context_handoff",
    "device",
    "device_access_token",
    "device_authorization_receipt",
    "device_proof_nonce",
    "device_refresh_token",
    "device_token_family",
    "matrix_projection_cursor",
    "matrix_projection_event_receipt",
    "moderation_action",
    "moderation_case",
    "oidc_login_attempt",
    "outbox_event",
    "principal",
    "room_catalog_entry",
    "room_instance",
    "room_membership_projection",
    "web_session",
];

struct TestDatabase {
    migration: PgPool,
    runtime: PgPool,
}

impl TestDatabase {
    async fn connect() -> Self {
        let migration_url = required_url("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL");
        let runtime_url = required_url("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL");
        let migration = connect_pool(&migration_url).await;
        run_migrations(&migration).await.expect("首次迁移必须成功");
        let runtime = connect_pool(&runtime_url).await;
        Self { migration, runtime }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.migration.close().await;
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 迁移可重复执行且运行时角色没有建表权限() {
    let database = TestDatabase::connect().await;

    run_migrations(&database.migration)
        .await
        .expect("重复迁移必须幂等");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'agent_room' ORDER BY table_name",
    )
    .fetch_all(&database.migration)
    .await
    .expect("应能读取迁移结果");
    assert_eq!(tables, EXPECTED_TABLES);

    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&database.runtime)
        .await
        .expect("运行时连接应可用");
    assert_eq!(current_user, "agent_room_runtime");
    let can_create: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user, 'agent_room', 'CREATE')")
            .fetch_one(&database.runtime)
            .await
            .expect("应能检查角色权限");
    assert!(!can_create);

    let error = sqlx::query("CREATE TABLE agent_room.runtime_forbidden_table (id integer)")
        .execute(&database.runtime)
        .await
        .expect_err("运行时角色不得执行 DDL");
    assert_eq!(database_code(&error).as_deref(), Some("42501"));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 仓储执行真实读写并拒绝并发覆盖() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let registration = principal_registration(principal_id, "并发测试主体");

    let created = PrincipalRepository::create(&repositories, &registration)
        .await
        .expect("主体创建应成功");
    assert_eq!(
        PrincipalRepository::find(&repositories, principal_id)
            .await
            .expect("主体读取应成功"),
        Some(created.clone())
    );

    let mut first_writer = created.clone();
    let mut second_writer = created;
    first_writer.suspend().expect("状态转换应成功");
    second_writer.suspend().expect("状态转换应成功");
    let (first_result, second_result) = tokio::join!(
        PrincipalRepository::save(&repositories, &first_writer),
        PrincipalRepository::save(&repositories, &second_writer)
    );
    let outcomes = [first_result, second_result];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    let conflict = outcomes
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("另一个写入必须失败");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);

    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let agent_registration = AgentRegistration {
        agent: Agent::register(agent_id),
        owner_id: principal_id,
        matrix_user_id: format!("@agent-{agent_id}:matrix.agent-room.localhost"),
        slug: format!("agent-{agent_id}"),
        display_name: "仓储测试 Agent".to_owned(),
        description: "真实 PostgreSQL 事务测试".to_owned(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: test_time(),
    };
    let created_agent = AgentRepository::create(&repositories, &agent_registration)
        .await
        .expect("Agent 与 Owner 必须在同一事务中创建");
    assert_eq!(
        AgentRepository::find(&repositories, agent_id)
            .await
            .expect("Agent 读取应成功"),
        Some(created_agent)
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 事务失败会回滚且外键拒绝孤儿记录() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let missing_owner = PrincipalId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let registration = AgentRegistration {
        agent: Agent::register(agent_id),
        owner_id: missing_owner,
        matrix_user_id: format!("@rollback-{agent_id}:matrix.agent-room.localhost"),
        slug: format!("rollback-{agent_id}"),
        display_name: "回滚测试 Agent".to_owned(),
        description: String::new(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: test_time(),
    };

    let error = AgentRepository::create(&repositories, &registration)
        .await
        .expect_err("不存在的 Owner 必须导致事务失败");
    assert_eq!(error.kind(), RepositoryErrorKind::Constraint);
    let persisted: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_room.agent WHERE id = $1")
        .bind(agent_id.as_uuid())
        .fetch_one(&database.runtime)
        .await
        .expect("应能验证回滚结果");
    assert_eq!(persisted, 0);

    let orphan_error = sqlx::query(
        "INSERT INTO agent_room.device (\
            id, principal_id, label, platform, public_signing_key, trust_state, created_at\
         ) VALUES ($1, $2, '孤儿设备', 'windows', $3, 'pending', clock_timestamp())",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .bind(vec![7_u8; 32])
    .execute(&database.runtime)
    .await
    .expect_err("孤儿设备必须被外键拒绝");
    assert_eq!(database_code(&orphan_error).as_deref(), Some("23503"));

    let rolled_back_id = PrincipalId::from_uuid(Uuid::now_v7());
    let mut transaction = database.runtime.begin().await.expect("应能开启显式事务");
    sqlx::query(
        "INSERT INTO agent_room.principal (\
            id, oidc_issuer, oidc_subject, matrix_user_id, display_name, locale,\
            status, created_at, updated_at, version\
         ) VALUES ($1, 'https://issuer.test', $2, $3, '回滚主体', 'zh-CN',\
            'active', clock_timestamp(), clock_timestamp(), 0)",
    )
    .bind(rolled_back_id.as_uuid())
    .bind(format!("subject-{rolled_back_id}"))
    .bind(format!(
        "@rollback-{rolled_back_id}:matrix.agent-room.localhost"
    ))
    .execute(&mut *transaction)
    .await
    .expect("事务内写入应成功");
    transaction.rollback().await.expect("显式回滚应成功");
    let persisted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.principal WHERE id = $1")
            .bind(rolled_back_id.as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("应能读取回滚后的状态");
    assert_eq!(persisted, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 审计日志只允许追加写入() {
    let database = TestDatabase::connect().await;
    let event_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agent_room.audit_event (\
            id, occurred_at, actor_kind, actor_reference, action, target_kind,\
            target_reference, outcome, correlation_id, metadata\
         ) VALUES ($1, clock_timestamp(), 'service', 'integration-test',\
            'database.verify', 'database', 'agent-room', 'allowed', $2, '{}'::jsonb)",
    )
    .bind(event_id)
    .bind(Uuid::now_v7())
    .execute(&database.runtime)
    .await
    .expect("运行时角色应能追加审计事件");

    let permission_error =
        sqlx::query("UPDATE agent_room.audit_event SET outcome = 'failed' WHERE id = $1")
            .bind(event_id)
            .execute(&database.runtime)
            .await
            .expect_err("运行时角色没有审计更新权限");
    assert_eq!(database_code(&permission_error).as_deref(), Some("42501"));

    let trigger_error =
        sqlx::query("UPDATE agent_room.audit_event SET outcome = 'failed' WHERE id = $1")
            .bind(event_id)
            .execute(&database.migration)
            .await
            .expect_err("迁移所有者也不得篡改审计事件");
    assert_eq!(database_code(&trigger_error).as_deref(), Some("55000"));

    database.close().await;
}

fn principal_registration(id: PrincipalId, display_name: &str) -> PrincipalRegistration {
    PrincipalRegistration {
        principal: Principal::new(id),
        oidc_issuer: "https://issuer.test".to_owned(),
        oidc_subject: format!("subject-{id}"),
        matrix_user_id: format!("@principal-{id}:matrix.agent-room.localhost"),
        display_name: display_name.to_owned(),
        avatar_content_id: None,
        locale: "zh-CN".to_owned(),
        registered_at: test_time(),
    }
}

fn test_time() -> UtcMillis {
    UtcMillis::new(1_700_000_000_000).expect("测试时间戳必须有效")
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

fn database_code(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(database_error) => database_error.code().map(Cow::into_owned),
        _ => None,
    }
}
