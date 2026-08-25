use std::env;

use agent_room_application::ports::{
    DirectSessionAgentDirectory, DirectSessionRecord, DirectSessionStore,
};
use agent_room_domain::{
    direct_sessions::DirectSession,
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomCatalogVisibility,
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
async fn 直接会话重复预留复用同一记录并完整往返屏蔽事实() {
    let database = TestDatabase::connect().await;
    let principal_id = seed_principal(&database.runtime).await;
    let agent_id = seed_agent(&database.runtime, principal_id).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());

    let profile =
        DirectSessionAgentDirectory::find_contactable(&repositories, principal_id, agent_id)
            .await
            .expect("Agent 目录读取应成功")
            .expect("公开 Agent 可联系");
    assert_eq!(profile.display_name, "直接会话目标");

    let first = reservation(principal_id, agent_id);
    let first_catalog = first.catalog().id();
    let reserved = DirectSessionStore::reserve(&repositories, &first, time(0))
        .await
        .expect("首次预留应成功");
    assert_eq!(reserved.catalog().id(), first_catalog);

    let duplicate = reservation(principal_id, agent_id);
    let duplicate_catalog = duplicate.catalog().id();
    let reused = DirectSessionStore::reserve(&repositories, &duplicate, time(1))
        .await
        .expect("重复预留应复用");
    assert_eq!(reused.catalog().id(), first_catalog);
    let duplicate_orphan: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_room.room_catalog_entry WHERE id = $1)",
    )
    .bind(duplicate_catalog.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("可检查冲突目录");
    assert!(!duplicate_orphan, "冲突预留不得遗留孤儿目录");

    let expected = reserved.session().version();
    let room_reference = MatrixRoomReference::new(format!(
        "!direct{}:matrix.test",
        first_catalog.as_uuid().simple()
    ))
    .expect("Matrix 房间标识有效");
    let active = reserved
        .activate(
            RoomInstanceId::from_uuid(Uuid::now_v7()),
            room_reference.clone(),
        )
        .expect("预留记录可激活");
    DirectSessionStore::activate(&repositories, &active, expected, time(2))
        .await
        .expect("激活应原子保存目录和实例");

    let restored = DirectSessionStore::find_by_matrix_room(&repositories, &room_reference)
        .await
        .expect("按 Matrix 房间读取应成功")
        .expect("活动会话存在");
    assert_eq!(restored.catalog().id(), first_catalog);
    assert!(restored.session().is_active());
    assert_eq!(
        DirectSessionStore::list_for_principal(&repositories, principal_id)
            .await
            .expect("会话列表可读")
            .len(),
        1
    );

    let blocked = DirectSessionStore::set_principal_block(
        &repositories,
        principal_id,
        agent_id,
        true,
        time(3),
    )
    .await
    .expect("屏蔽应成功");
    assert!(!blocked.delivery_allowed());
    let blocked_again = DirectSessionStore::set_principal_block(
        &repositories,
        principal_id,
        agent_id,
        true,
        time(4),
    )
    .await
    .expect("重复屏蔽应幂等");
    assert!(blocked_again.principal_blocks_agent());
    let unblocked = DirectSessionStore::set_principal_block(
        &repositories,
        principal_id,
        agent_id,
        false,
        time(5),
    )
    .await
    .expect("解除屏蔽应成功");
    assert!(unblocked.delivery_allowed());

    database.close().await;
}

fn reservation(principal_id: PrincipalId, agent_id: AgentId) -> DirectSessionRecord {
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let catalog = RoomCatalog::new(
        catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::Direct,
            slug: None,
            name: "直接会话目标".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(principal_id),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: RoomCatalogStatus::Frozen,
        },
    )
    .expect("直接会话目录有效");
    DirectSessionRecord::new(
        catalog,
        None,
        DirectSession::reserve(catalog_id, principal_id, agent_id),
    )
    .expect("预留记录有效")
}

async fn seed_principal(pool: &PgPool) -> PrincipalId {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES (
               $1, 'https://issuer.test', $2, $3, '直接会话主体',
               'zh-CN', 'active',
               to_timestamp($4::double precision / 1000.0),
               to_timestamp($4::double precision / 1000.0), 0
           )",
    )
    .bind(principal_id.as_uuid())
    .bind(format!("direct-{}", principal_id.as_uuid().simple()))
    .bind(format!(
        "@user-{}:matrix.test",
        principal_id.as_uuid().simple()
    ))
    .bind(time(0).value())
    .execute(pool)
    .await
    .expect("主体写入应成功");
    principal_id
}

async fn seed_agent(pool: &PgPool, owner_id: PrincipalId) -> AgentId {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let mut transaction = pool.begin().await.expect("Agent 事务应启动");
    sqlx::query(
        r"INSERT INTO agent_room.agent (
               id, matrix_user_id, slug, display_name, description, visibility,
               lifecycle_state, created_at, updated_at, version
           ) VALUES (
               $1, $2, $3, '直接会话目标', '', 'public', 'active',
               to_timestamp($4::double precision / 1000.0),
               to_timestamp($4::double precision / 1000.0), 0
           )",
    )
    .bind(agent_id.as_uuid())
    .bind(format!(
        "@_agent_{}:matrix.test",
        agent_id.as_uuid().simple()
    ))
    .bind(format!("direct-{}", agent_id.as_uuid().simple()))
    .bind(time(0).value())
    .execute(&mut *transaction)
    .await
    .expect("Agent 写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
               principal_id, agent_id, role, granted_by, created_at
           ) VALUES (
               $1, $2, 'owner', $1,
               to_timestamp($3::double precision / 1000.0)
           )",
    )
    .bind(owner_id.as_uuid())
    .bind(agent_id.as_uuid())
    .bind(time(0).value())
    .execute(&mut *transaction)
    .await
    .expect("Agent Owner 写入应成功");
    transaction.commit().await.expect("Agent 事务应提交");
    agent_id
}

fn time(offset: i64) -> UtcMillis {
    UtcMillis::new(1_800_000_000_000 + offset).expect("测试时间有效")
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
