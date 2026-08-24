use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        RoomAllocationEvidence, RoomAllocationMode, RoomAllocationStore, RoomDirectory,
        RoomDirectoryQuery, RoomReservationClaim, RoomReservationOutcome,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{RoomLanguage, RoomRegion, RoomReservationState},
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
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

struct Fixture {
    agent_id: AgentId,
    agent_instances: Vec<AgentInstanceId>,
    catalog_id: RoomCatalogId,
    room_ids: Vec<RoomInstanceId>,
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 并发手动分房绝不突破硬容量() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime, 8, 1, 1, 3).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let room_id = fixture.room_ids[0];
    let mut tasks = JoinSet::new();

    for (offset, agent_instance_id) in fixture.agent_instances.iter().copied().enumerate() {
        let repositories = repositories.clone();
        let claim = claim(
            fixture.agent_id,
            agent_instance_id,
            fixture.catalog_id,
            RoomAllocationMode::Manual(room_id),
            i64::try_from(offset).expect("测试偏移量受控"),
        );
        tasks.spawn(async move { RoomAllocationStore::reserve(&repositories, &claim).await });
    }

    let mut reserved = 0;
    let mut rejected = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("并发任务不能崩溃") {
            Ok(RoomReservationOutcome::Reserved { .. }) => reserved += 1,
            Err(error) if error.kind() == RepositoryErrorKind::Constraint => rejected += 1,
            unexpected => panic!("出现非预期分配结果：{unexpected:?}"),
        }
    }
    assert_eq!(reserved, 3);
    assert_eq!(rejected, 5);
    assert_eq!(allocated_slots(&database.runtime, room_id).await, 3);
    assert_eq!(active_reservations(&database.runtime, room_id).await, 3);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 手动换房会原子确认新房并释放旧房() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime, 1, 2, 2, 4).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let agent_instance_id = fixture.agent_instances[0];
    let first = claim(
        fixture.agent_id,
        agent_instance_id,
        fixture.catalog_id,
        RoomAllocationMode::Manual(fixture.room_ids[0]),
        0,
    );
    let first_reservation = expect_reserved(
        RoomAllocationStore::reserve(&repositories, &first)
            .await
            .expect("首次预约应成功"),
    );
    RoomAllocationStore::transition(
        &repositories,
        first_reservation,
        RoomReservationState::Reserved,
        RoomReservationState::Committed,
        test_time(100),
    )
    .await
    .expect("首次 Matrix 加入后应确认");

    let second = claim(
        fixture.agent_id,
        agent_instance_id,
        fixture.catalog_id,
        RoomAllocationMode::Manual(fixture.room_ids[1]),
        200,
    );
    let second_reservation = expect_reserved(
        RoomAllocationStore::reserve(&repositories, &second)
            .await
            .expect("换房预约应成功"),
    );
    RoomAllocationStore::transition(
        &repositories,
        second_reservation,
        RoomReservationState::Reserved,
        RoomReservationState::Committed,
        test_time(300),
    )
    .await
    .expect("新房 Matrix 加入后应原子切换归属");

    assert_eq!(
        allocated_slots(&database.runtime, fixture.room_ids[0]).await,
        0
    );
    assert_eq!(
        allocated_slots(&database.runtime, fixture.room_ids[1]).await,
        1
    );
    assert_eq!(
        reservation_state(&database.runtime, first_reservation).await,
        "released"
    );
    assert_eq!(
        reservation_state(&database.runtime, second_reservation).await,
        "committed"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 到期回收只释放未确认预约() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime, 1, 1, 2, 4).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let reservation_id = expect_reserved(
        RoomAllocationStore::reserve(
            &repositories,
            &claim(
                fixture.agent_id,
                fixture.agent_instances[0],
                fixture.catalog_id,
                RoomAllocationMode::Manual(fixture.room_ids[0]),
                0,
            ),
        )
        .await
        .expect("预约应成功"),
    );

    let expired = RoomAllocationStore::expire_pending(&repositories, test_time(60_000), 32)
        .await
        .expect("到期扫描应成功");
    assert_eq!(expired, 1);
    assert_eq!(
        allocated_slots(&database.runtime, fixture.room_ids[0]).await,
        0
    );
    assert_eq!(
        reservation_state(&database.runtime, reservation_id).await,
        "expired"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 达到软容量时自动分房明确要求新建实例且不伪造预约() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime, 1, 1, 1, 3).await;
    sqlx::query("UPDATE agent_room.room_instance SET allocated_slots = 1 WHERE id = $1")
        .bind(fixture.room_ids[0].as_uuid())
        .execute(&database.runtime)
        .await
        .expect("测试应能预置软容量");
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let outcome = RoomAllocationStore::reserve(
        &repositories,
        &claim(
            fixture.agent_id,
            fixture.agent_instances[0],
            fixture.catalog_id,
            RoomAllocationMode::Automatic,
            0,
        ),
    )
    .await
    .expect("自动分配查询应成功");

    assert!(matches!(
        outcome,
        RoomReservationOutcome::ProvisioningRequired { .. }
    ));
    assert_eq!(
        reservation_count(&database.runtime, fixture.catalog_id).await,
        0
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 公开目录按语言地区过滤并汇总活跃实例() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime, 1, 2, 2, 4).await;
    sqlx::query(
        "UPDATE agent_room.room_instance \
         SET member_count_projection = 2, activity_score = 1.2500 \
         WHERE catalog_entry_id = $1",
    )
    .bind(fixture.catalog_id.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("测试应能写入目录投影");
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let entries = RoomDirectory::list_public(
        &repositories,
        &RoomDirectoryQuery {
            language: Some(RoomLanguage::new("zh-CN").expect("语言有效")),
            region: Some(RoomRegion::new("ap-southeast").expect("地区有效")),
        },
    )
    .await
    .expect("目录查询应成功");
    let entry = entries
        .iter()
        .find(|entry| entry.catalog.id() == fixture.catalog_id)
        .expect("测试大厅应出现在目录中");
    assert_eq!(entry.active_instance_count, 2);
    assert_eq!(entry.online_agent_count, 4);
    assert_eq!(entry.activity_score_millis, 2_500);

    database.close().await;
}

fn claim(
    agent_id: AgentId,
    agent_instance_id: AgentInstanceId,
    catalog_id: RoomCatalogId,
    mode: RoomAllocationMode,
    offset: i64,
) -> RoomReservationClaim {
    RoomReservationClaim {
        reservation_id: RoomReservationId::from_uuid(Uuid::now_v7()),
        agent_id,
        agent_instance_id,
        catalog_id,
        mode,
        preferred_language: Some(RoomLanguage::new("zh-CN").expect("语言有效")),
        preferred_region: Some(RoomRegion::new("ap-southeast").expect("地区有效")),
        evidence: RoomAllocationEvidence::default(),
        reserved_at: test_time(offset),
        expires_at: test_time(offset + 60_000),
    }
}

fn expect_reserved(outcome: RoomReservationOutcome) -> RoomReservationId {
    match outcome {
        RoomReservationOutcome::Reserved { reservation, .. } => reservation.id(),
        unexpected => panic!("预期获得容量预约，实际为 {unexpected:?}"),
    }
}

async fn seed_fixture(
    pool: &PgPool,
    instance_count: usize,
    room_count: usize,
    soft_capacity: i32,
    hard_capacity: i32,
) -> Fixture {
    let principal_id = Uuid::now_v7();
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let suffix = agent_id.as_uuid().simple().to_string();
    let mut transaction = pool.begin().await.expect("测试事务应启动");
    insert_principal_and_agent(&mut transaction, principal_id, agent_id, &suffix).await;
    let agent_instances = insert_agent_instances(
        &mut transaction,
        principal_id,
        agent_id,
        instance_count,
        &suffix,
    )
    .await;
    let room_ids = insert_lobby(
        &mut transaction,
        catalog_id,
        room_count,
        soft_capacity,
        hard_capacity,
        &suffix,
    )
    .await;
    transaction.commit().await.expect("测试夹具应原子提交");
    Fixture {
        agent_id,
        agent_instances,
        catalog_id,
        room_ids,
    }
}

async fn insert_principal_and_agent(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: Uuid,
    agent_id: AgentId,
    suffix: &str,
) {
    sqlx::query(
        r"INSERT INTO agent_room.principal (
              id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
              locale, status, created_at, updated_at
          ) VALUES (
              $1, 'https://issuer.test', $2, $3, '房间测试主体',
              'zh-CN', 'active', to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(principal_id)
    .bind(format!("subject-{suffix}"))
    .bind(format!("@principal-{suffix}:matrix.test"))
    .execute(&mut **transaction)
    .await
    .expect("测试主体应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent (
              id, matrix_user_id, slug, display_name, visibility, lifecycle_state,
              created_at, updated_at
          ) VALUES (
              $1, $2, $3, '房间测试 Agent', 'public', 'active',
              to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(agent_id.as_uuid())
    .bind(format!("@agent-{suffix}:matrix.test"))
    .bind(format!("agent-{}", &suffix[..24]))
    .execute(&mut **transaction)
    .await
    .expect("测试 Agent 应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
              principal_id, agent_id, role, granted_by, created_at
          ) VALUES ($1, $2, 'owner', $1, to_timestamp(1700000000))",
    )
    .bind(principal_id)
    .bind(agent_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .expect("测试 Owner 应创建");
}

async fn insert_agent_instances(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: Uuid,
    agent_id: AgentId,
    count: usize,
    suffix: &str,
) -> Vec<AgentInstanceId> {
    let mut instances = Vec::with_capacity(count);
    for index in 0..count {
        let device_id = Uuid::now_v7();
        let binding_id = Uuid::now_v7();
        let instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
        sqlx::query(
            r"INSERT INTO agent_room.device (
                  id, principal_id, label, platform, public_signing_key,
                  trust_state, verified_at, created_at
              ) VALUES (
                  $1, $2, $3, 'windows', $4, 'verified',
                  to_timestamp(1700000000), to_timestamp(1700000000)
              )",
        )
        .bind(device_id)
        .bind(principal_id)
        .bind(format!("测试设备 {index}"))
        .bind(test_key(device_id))
        .execute(&mut **transaction)
        .await
        .expect("测试设备应创建");
        sqlx::query(
            r"INSERT INTO agent_room.adapter_binding (
                  id, agent_id, adapter_type, external_subject_hash,
                  capability_version, state, created_at, updated_at
              ) VALUES (
                  $1, $2, 'codex', $3, '1.0', 'active',
                  to_timestamp(1700000000), to_timestamp(1700000000)
              )",
        )
        .bind(binding_id)
        .bind(agent_id.as_uuid())
        .bind(test_key(binding_id))
        .execute(&mut **transaction)
        .await
        .expect("测试适配器绑定应创建");
        sqlx::query(
            r"INSERT INTO agent_room.agent_instance (
                  id, agent_id, device_id, adapter_binding_id, public_signing_key,
                  matrix_device_id, status, created_at
              ) VALUES (
                  $1, $2, $3, $4, $5, $6, 'connecting', to_timestamp(1700000000)
              )",
        )
        .bind(instance_id.as_uuid())
        .bind(agent_id.as_uuid())
        .bind(device_id)
        .bind(binding_id)
        .bind(test_key(instance_id.as_uuid()))
        .bind(format!("AR_{index}_{suffix}"))
        .execute(&mut **transaction)
        .await
        .expect("测试 Agent 实例应创建");
        instances.push(instance_id);
    }
    instances
}

fn test_key(id: Uuid) -> Vec<u8> {
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(id.as_bytes());
    key.extend_from_slice(id.as_bytes());
    key
}

async fn insert_lobby(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
    room_count: usize,
    soft_capacity: i32,
    hard_capacity: i32,
    suffix: &str,
) -> Vec<RoomInstanceId> {
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
              id, kind, slug, name, language, visibility, status, created_at, updated_at
          ) VALUES (
              $1, 'public_lobby', $2, '公开测试大厅', 'zh-CN', 'public', 'active',
              to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(catalog_id.as_uuid())
    .bind(format!("lobby-{}", &suffix[..24]))
    .execute(&mut **transaction)
    .await
    .expect("测试大厅目录应创建");
    let mut rooms = Vec::with_capacity(room_count);
    for index in 0..room_count {
        let room_id = RoomInstanceId::from_uuid(Uuid::now_v7());
        sqlx::query(
            r"INSERT INTO agent_room.room_instance (
                  id, catalog_entry_id, matrix_room_id, region_hint,
                  soft_capacity, hard_capacity, state, created_at, updated_at
              ) VALUES (
                  $1, $2, $3, 'ap-southeast', $4, $5, 'active',
                  to_timestamp(1700000000), to_timestamp(1700000000)
              )",
        )
        .bind(room_id.as_uuid())
        .bind(catalog_id.as_uuid())
        .bind(format!("!room-{index}-{suffix}:matrix.test"))
        .bind(soft_capacity)
        .bind(hard_capacity)
        .execute(&mut **transaction)
        .await
        .expect("测试房间实例应创建");
        rooms.push(room_id);
    }
    rooms
}

async fn allocated_slots(pool: &PgPool, room_id: RoomInstanceId) -> i32 {
    sqlx::query_scalar("SELECT allocated_slots FROM agent_room.room_instance WHERE id = $1")
        .bind(room_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("应能读取槽位")
}

async fn active_reservations(pool: &PgPool, room_id: RoomInstanceId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.room_capacity_reservation \
         WHERE room_instance_id = $1 AND state IN ('reserved', 'committed')",
    )
    .bind(room_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能统计活跃预约")
}

async fn reservation_state(pool: &PgPool, reservation_id: RoomReservationId) -> String {
    sqlx::query_scalar("SELECT state FROM agent_room.room_capacity_reservation WHERE id = $1")
        .bind(reservation_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("应能读取预约状态")
}

async fn reservation_count(pool: &PgPool, catalog_id: RoomCatalogId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.room_capacity_reservation WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能统计预约")
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
        .max_connections(16)
        .connect(url)
        .await
        .expect("真实 PostgreSQL 必须可连接")
}
