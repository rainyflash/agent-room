use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        MatrixRoomAliasLocalpart, RoomProvisioningClaim, RoomProvisioningClaimOutcome,
        RoomProvisioningFailureCode, RoomProvisioningJob, RoomProvisioningStore,
        RoomProvisioningTarget,
    },
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId, RoomProvisioningJobId, RoomProvisioningLeaseId},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomInstance, RoomInstanceFields, RoomInstanceState,
        RoomRegion,
    },
    time::UtcMillis,
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
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
async fn 并发建房声明只产生一个待办任务() {
    let database = TestDatabase::connect().await;
    let catalog_id = seed_catalog(&database.runtime, false).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let alias = space_alias(catalog_id);
    let mut tasks = JoinSet::new();

    for offset in 0..2 {
        let repositories = repositories.clone();
        let claim = provisioning_claim(
            catalog_id,
            RoomProvisioningTarget::Space,
            alias.clone(),
            offset,
            60_000,
        );
        tasks.spawn(async move { RoomProvisioningStore::claim(&repositories, &claim).await });
    }

    let mut claimed = 0;
    let mut busy = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("并发任务不能崩溃").expect("声明查询应成功") {
            RoomProvisioningClaimOutcome::Claimed(_) => claimed += 1,
            RoomProvisioningClaimOutcome::Busy { .. } => busy += 1,
            unexpected => panic!("出现非预期声明结果：{unexpected:?}"),
        }
    }

    assert_eq!(claimed, 1);
    assert_eq!(busy, 1);
    assert_eq!(pending_job_count(&database.runtime, catalog_id).await, 1);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 过期租约接管保留矩阵断点并拒绝旧持有者() {
    let database = TestDatabase::connect().await;
    let catalog_id = seed_catalog(&database.runtime, false).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let first_claim = provisioning_claim(
        catalog_id,
        RoomProvisioningTarget::Space,
        space_alias(catalog_id),
        0,
        100,
    );
    let first_job = expect_claimed(
        RoomProvisioningStore::claim(&repositories, &first_claim)
            .await
            .expect("首次声明应成功"),
    );
    let matrix_space_id = MatrixRoomReference::new("!resume-space:matrix.test").expect("房间有效");
    RoomProvisioningStore::checkpoint_matrix_room(
        &repositories,
        &first_job,
        &matrix_space_id,
        test_time(10),
    )
    .await
    .expect("应持久化 Matrix 断点");

    let takeover_claim = RoomProvisioningClaim::new(
        RoomProvisioningJobId::from_uuid(Uuid::now_v7()),
        RoomProvisioningLeaseId::from_uuid(Uuid::now_v7()),
        catalog_id,
        RoomProvisioningTarget::Space,
        MatrixRoomAliasLocalpart::new("replacement-space-alias").expect("备用别名有效"),
        test_time(101),
        test_time(300),
    )
    .expect("接管声明有效");
    let takeover_job = expect_claimed(
        RoomProvisioningStore::claim(&repositories, &takeover_claim)
            .await
            .expect("过期租约应可接管"),
    );

    assert_eq!(takeover_job.job_id(), first_job.job_id());
    assert_eq!(takeover_job.alias_localpart(), first_job.alias_localpart());
    assert_eq!(takeover_job.matrix_room_id(), Some(&matrix_space_id));
    assert_eq!(takeover_job.lease_id(), takeover_claim.lease_id());

    let stale_error = RoomProvisioningStore::complete_space(
        &repositories,
        &first_job,
        &matrix_space_id,
        test_time(120),
    )
    .await
    .expect_err("旧 fencing token 不得提交");
    assert_eq!(stale_error.kind(), RepositoryErrorKind::Conflict);

    let catalog = RoomProvisioningStore::complete_space(
        &repositories,
        &takeover_job,
        &matrix_space_id,
        test_time(120),
    )
    .await
    .expect("新租约应能提交");
    assert_eq!(catalog.matrix_space_id(), Some(&matrix_space_id));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 释放失败任务后可立即续作同一任务() {
    let database = TestDatabase::connect().await;
    let catalog_id = seed_catalog(&database.runtime, false).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let first_claim = provisioning_claim(
        catalog_id,
        RoomProvisioningTarget::Space,
        space_alias(catalog_id),
        0,
        60_000,
    );
    let first_job = expect_claimed(
        RoomProvisioningStore::claim(&repositories, &first_claim)
            .await
            .expect("首次声明应成功"),
    );
    RoomProvisioningStore::release(
        &repositories,
        &first_job,
        RoomProvisioningFailureCode::MatrixCreate,
        test_time(10),
    )
    .await
    .expect("失败任务应释放租约");

    let second_claim = provisioning_claim(
        catalog_id,
        RoomProvisioningTarget::Space,
        MatrixRoomAliasLocalpart::new("replacement-space-alias").expect("备用别名有效"),
        11,
        60_000,
    );
    let resumed_job = expect_claimed(
        RoomProvisioningStore::claim(&repositories, &second_claim)
            .await
            .expect("释放后应立即可续作"),
    );

    assert_eq!(resumed_job.job_id(), first_job.job_id());
    assert_eq!(resumed_job.alias_localpart(), first_job.alias_localpart());
    assert_eq!(resumed_job.lease_id(), second_claim.lease_id());
    assert_eq!(
        provisioning_failure(&database.runtime, first_job.job_id()).await,
        None
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 发布空间和实例后后续声明直接复用现有资源() {
    let database = TestDatabase::connect().await;
    let catalog_id = seed_catalog(&database.runtime, false).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let space_claim = provisioning_claim(
        catalog_id,
        RoomProvisioningTarget::Space,
        space_alias(catalog_id),
        0,
        60_000,
    );
    let space_job = expect_claimed(
        RoomProvisioningStore::claim(&repositories, &space_claim)
            .await
            .expect("Space 声明应成功"),
    );
    let matrix_space_id = MatrixRoomReference::new("!catalog-space:matrix.test").expect("房间有效");
    checkpoint_and_complete_space(&repositories, &space_job, &matrix_space_id, test_time(10)).await;

    let ready_space = RoomProvisioningStore::claim(
        &repositories,
        &provisioning_claim(
            catalog_id,
            RoomProvisioningTarget::Space,
            space_alias(catalog_id),
            20,
            60_000,
        ),
    )
    .await
    .expect("已发布 Space 查询应成功");
    assert!(matches!(
        ready_space,
        RoomProvisioningClaimOutcome::SpaceReady { .. }
    ));

    let region = RoomRegion::new("ap-southeast").expect("地区有效");
    let room_instance_id = RoomInstanceId::from_uuid(Uuid::now_v7());
    let target = RoomProvisioningTarget::Instance {
        room_instance_id,
        region: Some(region.clone()),
    };
    let instance_claim = provisioning_claim(
        catalog_id,
        target.clone(),
        instance_alias(room_instance_id),
        30,
        60_000,
    );
    let instance_job = expect_claimed(
        RoomProvisioningStore::claim(&repositories, &instance_claim)
            .await
            .expect("实例声明应成功"),
    );
    let matrix_room_id = MatrixRoomReference::new("!lobby-instance:matrix.test").expect("房间有效");
    let room = RoomInstance::restore(
        room_instance_id,
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: matrix_room_id.clone(),
            region: Some(region),
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("实例快照有效");
    RoomProvisioningStore::checkpoint_matrix_room(
        &repositories,
        &instance_job,
        &matrix_room_id,
        test_time(40),
    )
    .await
    .expect("实例断点应保存");
    RoomProvisioningStore::complete_instance(&repositories, &instance_job, &room, test_time(50))
        .await
        .expect("实例应原子发布");

    let ready_instance = RoomProvisioningStore::claim(
        &repositories,
        &provisioning_claim(
            catalog_id,
            target,
            MatrixRoomAliasLocalpart::new("unused-instance-alias").expect("备用别名有效"),
            60,
            60_000,
        ),
    )
    .await
    .expect("已发布实例查询应成功");
    match ready_instance {
        RoomProvisioningClaimOutcome::InstanceReady { room: ready } => {
            assert_eq!(ready.id(), room_instance_id);
        }
        unexpected => panic!("预期复用实例，实际为 {unexpected:?}"),
    }
    assert_eq!(completed_job_count(&database.runtime, catalog_id).await, 2);

    database.close().await;
}

async fn checkpoint_and_complete_space(
    repositories: &PostgresRepositories,
    job: &RoomProvisioningJob,
    matrix_space_id: &MatrixRoomReference,
    changed_at: UtcMillis,
) {
    RoomProvisioningStore::checkpoint_matrix_room(repositories, job, matrix_space_id, changed_at)
        .await
        .expect("Space 断点应保存");
    RoomProvisioningStore::complete_space(repositories, job, matrix_space_id, changed_at)
        .await
        .expect("Space 应原子发布");
}

fn provisioning_claim(
    catalog_id: RoomCatalogId,
    target: RoomProvisioningTarget,
    alias: MatrixRoomAliasLocalpart,
    claimed_offset: i64,
    lease_duration: i64,
) -> RoomProvisioningClaim {
    RoomProvisioningClaim::new(
        RoomProvisioningJobId::from_uuid(Uuid::now_v7()),
        RoomProvisioningLeaseId::from_uuid(Uuid::now_v7()),
        catalog_id,
        target,
        alias,
        test_time(claimed_offset),
        test_time(claimed_offset + lease_duration),
    )
    .expect("建房声明有效")
}

fn expect_claimed(outcome: RoomProvisioningClaimOutcome) -> RoomProvisioningJob {
    match outcome {
        RoomProvisioningClaimOutcome::Claimed(job) => job,
        unexpected => panic!("预期获得建房租约，实际为 {unexpected:?}"),
    }
}

fn space_alias(catalog_id: RoomCatalogId) -> MatrixRoomAliasLocalpart {
    MatrixRoomAliasLocalpart::new(format!(
        "agent-room-space-{}",
        catalog_id.as_uuid().simple()
    ))
    .expect("Space 别名有效")
}

fn instance_alias(room_instance_id: RoomInstanceId) -> MatrixRoomAliasLocalpart {
    MatrixRoomAliasLocalpart::new(format!(
        "agent-room-instance-{}",
        room_instance_id.as_uuid().simple()
    ))
    .expect("实例别名有效")
}

async fn seed_catalog(pool: &PgPool, with_space: bool) -> RoomCatalogId {
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let suffix = catalog_id.as_uuid().simple().to_string();
    let matrix_space_id = with_space.then(|| format!("!space-{suffix}:matrix.test"));
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
              id, kind, slug, name, description, language, matrix_space_id,
              visibility, status, created_at, updated_at
          ) VALUES (
              $1, 'public_lobby', $2, '供给测试大厅', '验证可恢复建房流程', 'zh-CN', $3,
              'public', 'active', to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(catalog_id.as_uuid())
    .bind(format!("provisioning-{}", &suffix[..24]))
    .bind(matrix_space_id)
    .execute(pool)
    .await
    .expect("测试大厅目录应创建");
    catalog_id
}

async fn pending_job_count(pool: &PgPool, catalog_id: RoomCatalogId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.room_provisioning_job \
         WHERE catalog_entry_id = $1 AND state = 'pending'",
    )
    .bind(catalog_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能统计待办任务")
}

async fn completed_job_count(pool: &PgPool, catalog_id: RoomCatalogId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.room_provisioning_job \
         WHERE catalog_entry_id = $1 AND state = 'completed'",
    )
    .bind(catalog_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能统计完成任务")
}

async fn provisioning_failure(pool: &PgPool, job_id: RoomProvisioningJobId) -> Option<String> {
    sqlx::query_scalar("SELECT failure_code FROM agent_room.room_provisioning_job WHERE id = $1")
        .bind(job_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("应能读取失败码")
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
