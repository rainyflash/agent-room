use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{PrivateRoomSnapshot, PrivateRoomStore},
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus,
        PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
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
async fn 私人房间生命周期可完整往返且不依赖房主进程() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime, "owner").await;
    let member = seed_principal(&database.runtime, "member").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let snapshot = snapshot(catalog_id, owner);

    PrivateRoomStore::create(&repositories, &snapshot, time(0))
        .await
        .expect("创建应原子保存目录、实例和房主");
    let mut loaded = PrivateRoomStore::find_by_catalog(&repositories, catalog_id)
        .await
        .expect("读取应成功")
        .expect("私人房间应存在");

    let expected = loaded.room().version();
    let mut room = loaded.room().clone();
    room.invite(owner, member, ordinary_permissions())
        .expect("房主可邀请");
    PrivateRoomStore::save(&repositories, &room, expected, time(1))
        .await
        .expect("邀请应持久化");

    loaded =
        PrivateRoomStore::find_by_matrix_room(&repositories, snapshot.instance().matrix_room_id())
            .await
            .expect("按 Matrix 房间读取应成功")
            .expect("私人房间应存在");
    assert_eq!(
        loaded.room().member(member).expect("邀请事实存在").status(),
        PrivateRoomMembershipStatus::Invited
    );

    let expected = loaded.room().version();
    room = loaded.room().clone();
    room.accept_invitation(member).expect("成员可接受邀请");
    PrivateRoomStore::save(&repositories, &room, expected, time(2))
        .await
        .expect("接受邀请应持久化");

    let expected = room.version();
    room.transfer_ownership(owner, member, ordinary_permissions())
        .expect("房主可转移给已加入成员");
    PrivateRoomStore::save(&repositories, &room, expected, time(3))
        .await
        .expect("所有权转移应原子持久化");

    let expected = room.version();
    room.remove_member(member, owner)
        .expect("新房主可移除原房主");
    PrivateRoomStore::save(&repositories, &room, expected, time(4))
        .await
        .expect("成员移除应持久化");
    assert!(!room.allows(owner, PrivateRoomCapability::View));

    let expected = room.version();
    room.archive(member).expect("新房主可归档");
    PrivateRoomStore::save(&repositories, &room, expected, time(5))
        .await
        .expect("归档应持久化");

    drop(repositories);
    let restarted_process = PostgresRepositories::new(database.runtime.clone());
    let restored = PrivateRoomStore::find_by_catalog(&restarted_process, catalog_id)
        .await
        .expect("新进程可读取")
        .expect("房间不会随原房主进程消失");
    assert_eq!(restored.room().owner_principal_id(), member);
    assert_eq!(
        restored.room().status(),
        PrivateRoomLifecycleStatus::Archived
    );
    assert_eq!(restored.catalog().status(), RoomCatalogStatus::Archived);
    assert_eq!(restored.instance().state(), RoomInstanceState::Archived);
    assert!(!restored.room().allows(owner, PrivateRoomCapability::View));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 乐观锁拒绝并发覆盖且数据库约束拒绝伪造房主() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime, "race-owner").await;
    let first_member = seed_principal(&database.runtime, "race-first").await;
    let second_member = seed_principal(&database.runtime, "race-second").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let initial = snapshot(catalog_id, owner);
    PrivateRoomStore::create(&repositories, &initial, time(10))
        .await
        .expect("创建应成功");

    let persisted = PrivateRoomStore::find_by_catalog(&repositories, catalog_id)
        .await
        .expect("读取应成功")
        .expect("房间存在");
    let expected = persisted.room().version();
    let mut first = persisted.room().clone();
    let mut stale = persisted.room().clone();
    first
        .invite(owner, first_member, ordinary_permissions())
        .expect("首个邀请有效");
    stale
        .invite(owner, second_member, ordinary_permissions())
        .expect("并发快照内邀请有效");

    PrivateRoomStore::save(&repositories, &first, expected, time(11))
        .await
        .expect("首个写入应成功");
    let conflict = PrivateRoomStore::save(&repositories, &stale, expected, time(12))
        .await
        .expect_err("过期版本不得覆盖新状态");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);

    let persisted = PrivateRoomStore::find_by_catalog(&repositories, catalog_id)
        .await
        .expect("读取应成功")
        .expect("房间存在");
    assert!(persisted.room().member(first_member).is_some());
    assert!(persisted.room().member(second_member).is_none());

    let mut corrupt = database.runtime.begin().await.expect("事务应启动");
    sqlx::query(
        r"UPDATE agent_room.room_catalog_entry
           SET owner_principal_id = $2
           WHERE id = $1",
    )
    .bind(catalog_id.as_uuid())
    .bind(first_member.as_uuid())
    .execute(&mut *corrupt)
    .await
    .expect("延迟约束应在提交点校验");
    assert!(corrupt.commit().await.is_err());

    let mut public_escape = database.runtime.begin().await.expect("事务应启动");
    sqlx::query("UPDATE agent_room.room_catalog_entry SET kind = 'public_lobby' WHERE id = $1")
        .bind(catalog_id.as_uuid())
        .execute(&mut *public_escape)
        .await
        .expect("延迟约束应在提交点校验");
    assert!(public_escape.commit().await.is_err());

    let mut lifecycle_split = database.runtime.begin().await.expect("事务应启动");
    sqlx::query(
        r"UPDATE agent_room.room_instance
           SET state = 'archived'
           WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .execute(&mut *lifecycle_split)
    .await
    .expect("延迟约束应在提交点校验");
    assert!(lifecycle_split.commit().await.is_err());

    database.close().await;
}

fn snapshot(catalog_id: RoomCatalogId, owner: PrincipalId) -> PrivateRoomSnapshot {
    let catalog = RoomCatalog::new(
        catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "隔离项目室".to_owned(),
            description: "只对受邀成员开放".to_owned(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("私人目录有效");
    let instance_id = RoomInstanceId::from_uuid(Uuid::now_v7());
    let instance = RoomInstance::restore(
        instance_id,
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: MatrixRoomReference::new(format!(
                "!private{}:matrix.test",
                instance_id.as_uuid().simple()
            ))
            .expect("Matrix 房间标识有效"),
            region: None,
            capacity: RoomCapacity::new(8, 16).expect("私人房间容量有效"),
            projected_member_count: 1,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("私人实例有效");
    PrivateRoomSnapshot::new(catalog, instance, PrivateRoom::create(catalog_id, owner))
        .expect("私人房间快照有效")
}

async fn seed_principal(pool: &PgPool, suffix: &str) -> PrincipalId {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES (
               $1, 'https://issuer.test', $2, $3, $4, 'zh-CN', 'active',
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0), 0
           )",
    )
    .bind(principal_id.as_uuid())
    .bind(format!(
        "subject-{suffix}-{}",
        principal_id.as_uuid().simple()
    ))
    .bind(format!(
        "@{suffix}-{}:matrix.test",
        principal_id.as_uuid().simple()
    ))
    .bind(format!("测试主体 {suffix}"))
    .bind(time(0).value())
    .execute(pool)
    .await
    .expect("主体写入应成功");
    principal_id
}

fn ordinary_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .expect("普通成员权限有效")
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
