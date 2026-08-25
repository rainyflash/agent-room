use std::env;

use agent_room_application::ports::ContentPrincipalIdentityLookup;
use agent_room_domain::ids::{AgentId, PrincipalId};
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
async fn 只解析活跃主体且数据库拒绝损坏标识() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = seed_principal(&database.runtime, "@content_reader:matrix.test").await;
    let agent_id = seed_owned_agent(&database.runtime, principal_id).await;

    let principal =
        ContentPrincipalIdentityLookup::find_active_matrix_user(&repositories, principal_id)
            .await
            .expect("活跃主体查询成功")
            .expect("活跃主体必须有 Matrix 身份");
    assert_eq!(principal.as_str(), "@content_reader:matrix.test");
    let agent = ContentPrincipalIdentityLookup::find_active_agent_matrix_user(
        &repositories,
        principal_id,
        agent_id,
    )
    .await
    .expect("活跃 Agent 查询成功")
    .expect("Owner 可用其 Agent 身份");
    assert_eq!(
        agent.as_str(),
        format!("@content_agent_{}:matrix.test", agent_id.as_uuid().simple())
    );

    sqlx::query("UPDATE agent_room.agent SET lifecycle_state = 'suspended' WHERE id = $1")
        .bind(agent_id.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("测试 Agent 可暂停");
    assert!(
        ContentPrincipalIdentityLookup::find_active_agent_matrix_user(
            &repositories,
            principal_id,
            agent_id,
        )
        .await
        .expect("暂停 Agent 后查询成功")
        .is_none()
    );
    assert_eq!(
        ContentPrincipalIdentityLookup::find_active_matrix_user(&repositories, principal_id)
            .await
            .expect("主体身份查询成功")
            .expect("Agent 暂停不影响主体身份")
            .as_str(),
        "@content_reader:matrix.test"
    );

    sqlx::query("UPDATE agent_room.principal SET status = 'suspended' WHERE id = $1")
        .bind(principal_id.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("测试主体可暂停");
    assert!(
        ContentPrincipalIdentityLookup::find_active_matrix_user(&repositories, principal_id)
            .await
            .expect("暂停主体查询成功")
            .is_none()
    );
    assert!(
        ContentPrincipalIdentityLookup::find_active_agent_matrix_user(
            &repositories,
            principal_id,
            agent_id,
        )
        .await
        .expect("暂停主体的 Agent 查询成功")
        .is_none()
    );

    let failure = try_seed_principal(&database.runtime, "不是 Matrix 用户标识")
        .await
        .expect_err("数据库必须在身份边界拒绝损坏标识");
    match failure {
        sqlx::Error::Database(error) => {
            assert_eq!(error.code().as_deref(), Some("23514"));
        }
        other => panic!("预期数据库约束错误，实际为 {other:?}"),
    }

    database.close().await;
}

async fn seed_owned_agent(pool: &PgPool, principal_id: PrincipalId) -> AgentId {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.agent (
              id, matrix_user_id, slug, display_name, description, visibility,
              lifecycle_state, created_at, updated_at, version
          ) VALUES (
              $1, $2, $3, '内容身份测试 Agent', '', 'private',
              'active', to_timestamp(1), to_timestamp(1), 0
          )",
    )
    .bind(agent_id.as_uuid())
    .bind(format!(
        "@content_agent_{}:matrix.test",
        agent_id.as_uuid().simple()
    ))
    .bind(format!("content-agent-{}", agent_id.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("测试 Agent 写入成功");
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
              principal_id, agent_id, role, granted_by, created_at
          ) VALUES ($1, $2, 'owner', $1, to_timestamp(1))",
    )
    .bind(principal_id.as_uuid())
    .bind(agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("测试 Agent 所有权写入成功");
    agent_id
}

async fn seed_principal(pool: &PgPool, matrix_user_id: &str) -> PrincipalId {
    try_seed_principal(pool, matrix_user_id)
        .await
        .expect("测试主体写入成功")
}

async fn try_seed_principal(
    pool: &PgPool,
    matrix_user_id: &str,
) -> Result<PrincipalId, sqlx::Error> {
    let id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
              id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
              locale, status, created_at, updated_at, version
          ) VALUES (
              $1, 'https://issuer.content-identity.test', $2, $3, '内容身份测试主体',
              'zh-CN', 'active', to_timestamp(1), to_timestamp(1), 0
          )",
    )
    .bind(id.as_uuid())
    .bind(id.to_string())
    .bind(matrix_user_id)
    .execute(pool)
    .await?;
    Ok(id)
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
