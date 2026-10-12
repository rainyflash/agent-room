use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        NetworkAgentStore, PrivateRoomAgentDescription, PrivateRoomAgentDirectory,
        PrivateRoomPrincipalDirectory, PrivateRoomSnapshot, PrivateRoomStore,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    network_agents::NETWORK_AGENT_ISSUER,
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
    let owner_rooms = PrivateRoomStore::list_for_principal(&repositories, owner)
        .await
        .expect("房主目录读取应成功");
    assert_eq!(owner_rooms.len(), 1);
    assert_eq!(owner_rooms[0].catalog().id(), catalog_id);
    assert!(
        PrivateRoomStore::list_for_principal(&repositories, member)
            .await
            .expect("未受邀成员目录读取应成功")
            .is_empty()
    );
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
    let invited_rooms = PrivateRoomStore::list_for_principal(&repositories, member)
        .await
        .expect("受邀成员目录读取应成功");
    assert_eq!(invited_rooms.len(), 1);
    assert_eq!(invited_rooms[0].catalog().id(), catalog_id);
    assert_eq!(
        invited_rooms[0]
            .room()
            .member(member)
            .expect("受邀成员事实存在")
            .status(),
        PrivateRoomMembershipStatus::Invited
    );

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
    assert!(
        PrivateRoomStore::list_for_principal(&repositories, owner)
            .await
            .expect("已移除成员目录读取应成功")
            .is_empty(),
        "已移除成员不得继续从权威目录发现私人房间"
    );

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
async fn agent_建的房间记下建房的_agent_改动以后照旧在() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime, "agent-room-owner").await;
    let member = seed_principal(&database.runtime, "agent-room-member").await;
    let agent = seed_agent(&database.runtime, owner).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());

    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let created = snapshot_of(
        catalog_id,
        owner,
        PrivateRoom::create_by_agent(catalog_id, owner, agent),
    );
    PrivateRoomStore::create(&repositories, &created, time(0))
        .await
        .expect("Agent 建的房间应能保存");
    let loaded = PrivateRoomStore::find_by_catalog(&repositories, catalog_id)
        .await
        .expect("读取应成功")
        .expect("私人房间应存在");
    assert_eq!(loaded.room().created_by_agent(), Some(agent));

    let mut room = loaded.room().clone();
    room.invite(owner, member, ordinary_permissions())
        .expect("房主可邀请");
    PrivateRoomStore::save(&repositories, &room, loaded.room().version(), time(1))
        .await
        .expect("邀请应持久化");
    let saved = PrivateRoomStore::find_by_catalog(&repositories, catalog_id)
        .await
        .expect("读取应成功")
        .expect("私人房间应存在");
    assert_eq!(
        saved.room().created_by_agent(),
        Some(agent),
        "改动不碰建房的 Agent"
    );

    let person_catalog = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(&repositories, &snapshot(person_catalog, owner), time(2))
        .await
        .expect("人建的房间应能保存");
    let person_room = PrivateRoomStore::find_by_catalog(&repositories, person_catalog)
        .await
        .expect("读取应成功")
        .expect("私人房间应存在");
    assert_eq!(person_room.room().created_by_agent(), None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 只认主体名下还在用的_agent() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime, "agent-owner").await;
    let stranger = seed_principal(&database.runtime, "agent-stranger").await;
    let active = seed_agent(&database.runtime, owner).await;
    let suspended = seed_agent(&database.runtime, owner).await;
    sqlx::query("UPDATE agent_room.agent SET lifecycle_state = 'suspended' WHERE id = $1")
        .bind(suspended.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("停用 Agent 应成功");
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owns = |principal, agent| {
        let repositories = &repositories;
        async move {
            PrivateRoomAgentDirectory::owns_active_agent(repositories, principal, agent)
                .await
                .expect("所有权查询应成功")
        }
    };

    assert!(owns(owner, active).await);
    assert!(!owns(stranger, active).await, "别人的 Agent 不算");
    assert!(!owns(owner, suspended).await, "停用的 Agent 不算");
    assert!(
        !owns(owner, AgentId::from_uuid(Uuid::now_v7())).await,
        "不存在的 Agent 不算"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 建房的_agent_说得出名字_网络_agent_带着它的合成主体() {
    let database = TestDatabase::connect().await;
    let person = seed_principal(&database.runtime, "describe-person").await;
    let local = seed_agent(&database.runtime, person).await;
    let network =
        seed_principal_from(&database.runtime, "describe-network", NETWORK_AGENT_ISSUER).await;
    let remote = seed_agent(&database.runtime, network).await;
    seed_network_agent(&database.runtime, network, remote).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let describe = |agent| {
        let repositories = &repositories;
        async move {
            PrivateRoomAgentDirectory::describe_agent(repositories, agent)
                .await
                .expect("查询应成功")
        }
    };
    let described = |network_principal_id| {
        Some(PrivateRoomAgentDescription {
            display_name: "建房测试 Agent".to_owned(),
            network_principal_id,
        })
    };

    assert_eq!(describe(local).await, described(None));
    assert_eq!(describe(remote).await, described(Some(network)));
    sqlx::query("UPDATE agent_room.agent SET lifecycle_state = 'suspended' WHERE id = $1")
        .bind(remote.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("停用 Agent 应成功");
    assert_eq!(
        describe(remote).await,
        described(Some(network)),
        "停用了的照样回答：房间还在"
    );
    assert_eq!(describe(AgentId::from_uuid(Uuid::now_v7())).await, None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 全站网络_agent_当房主的没关的房间_人建的和关了的不算() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let count = || async {
        NetworkAgentStore::count_owned_private_rooms(&repositories)
            .await
            .expect("计数应成功")
    };
    // 别的测试也在同一个库里建房间，只看这里建的让它多了几个。
    let before = count().await;
    let network =
        seed_principal_from(&database.runtime, "count-network", NETWORK_AGENT_ISSUER).await;
    let remote = seed_agent(&database.runtime, network).await;
    seed_network_agent(&database.runtime, network, remote).await;
    let person = seed_principal(&database.runtime, "count-person").await;

    let mut rooms = Vec::new();
    for _ in 0..2 {
        let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
        PrivateRoomStore::create(
            &repositories,
            &snapshot_of(
                catalog_id,
                network,
                PrivateRoom::create_by_agent(catalog_id, network, remote),
            ),
            time(0),
        )
        .await
        .expect("网络 Agent 建的房间应能保存");
        rooms.push(catalog_id);
    }
    PrivateRoomStore::create(
        &repositories,
        &snapshot(RoomCatalogId::from_uuid(Uuid::now_v7()), person),
        time(0),
    )
    .await
    .expect("人建的房间应能保存");
    assert_eq!(count().await, before + 2, "人建的不算");

    let closed = PrivateRoomStore::find_by_catalog(&repositories, rooms[0])
        .await
        .expect("读取应成功")
        .expect("房间应存在");
    let mut room = closed.room().clone();
    assert!(room.archive(network).expect("房主可以关房间"));
    PrivateRoomStore::save(&repositories, &room, closed.room().version(), time(1))
        .await
        .expect("关房间应持久化");
    assert_eq!(count().await, before + 1, "关了的不算");

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 私人房间只把人当成员_网络_agent_的合成主体找不到() {
    let database = TestDatabase::connect().await;
    let person = seed_principal(&database.runtime, "directory-person").await;
    let network =
        seed_principal_from(&database.runtime, "directory-network", NETWORK_AGENT_ISSUER).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());

    let lookup = |principal| {
        let repositories = &repositories;
        async move {
            PrivateRoomPrincipalDirectory::matrix_user_id(repositories, principal)
                .await
                .expect("查 Matrix 账号应成功")
        }
    };
    assert!(lookup(person).await.is_some());
    assert!(
        lookup(network).await.is_none(),
        "合成主体没有 Matrix 账号，不能被当成人邀请"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 改名只改目录上的名字_不存在的房间不会悄悄成功() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime, "rename-owner").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let repositories = PostgresRepositories::new(database.runtime.clone());
    PrivateRoomStore::create(&repositories, &snapshot(catalog_id, owner), time(0))
        .await
        .expect("创建应成功");
    PrivateRoomStore::rename(&repositories, catalog_id, "Renamed room", time(1))
        .await
        .expect("改名应持久化");
    let renamed = PrivateRoomStore::find_by_catalog(&repositories, catalog_id)
        .await
        .expect("改名后可读取")
        .expect("房间仍在");
    assert_eq!(renamed.catalog().name(), "Renamed room");
    assert_eq!(renamed.room().owner_principal_id(), owner);
    assert_eq!(
        renamed.room().version(),
        snapshot(catalog_id, owner).room().version()
    );
    assert!(
        PrivateRoomStore::rename(
            &repositories,
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            "Nowhere",
            time(2),
        )
        .await
        .is_err()
    );
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
    snapshot_of(catalog_id, owner, PrivateRoom::create(catalog_id, owner))
}

fn snapshot_of(
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    room: PrivateRoom,
) -> PrivateRoomSnapshot {
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
    PrivateRoomSnapshot::new(catalog, instance, room).expect("私人房间快照有效")
}

/// 主体名下一个还在用的 Agent：先以停用状态写入，有了主人再激活（还在用的 Agent 必须有主人）。
async fn seed_agent(pool: &PgPool, owner: PrincipalId) -> AgentId {
    let agent = AgentId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.agent (
               id, matrix_user_id, slug, display_name, description, visibility,
               lifecycle_state, created_at, updated_at, version
            ) VALUES ($1, $2, $3, '建房测试 Agent', '', 'private',
                      'suspended', statement_timestamp(), statement_timestamp(), 0)",
    )
    .bind(agent.as_uuid())
    .bind(format!("@_agent_{}:matrix.test", agent.as_uuid().simple()))
    .bind(format!("agent-{}", agent.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("Agent 写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (
               principal_id, agent_id, role, granted_by, created_at
           ) VALUES ($1, $2, 'owner', $1, clock_timestamp())",
    )
    .bind(owner.as_uuid())
    .bind(agent.as_uuid())
    .execute(pool)
    .await
    .expect("Agent 所有权写入应成功");
    sqlx::query("UPDATE agent_room.agent SET lifecycle_state = 'active' WHERE id = $1")
        .bind(agent.as_uuid())
        .execute(pool)
        .await
        .expect("建立主人后应能激活 Agent");
    agent
}

/// 把合成主体和它的 Agent 连起来的网络 Agent 记录；停在 provisioning 就够了。
async fn seed_network_agent(pool: &PgPool, principal: PrincipalId, agent: AgentId) {
    let device = Uuid::now_v7();
    let digest = [
        principal.as_uuid().as_bytes().as_slice(),
        agent.as_uuid().as_bytes().as_slice(),
    ]
    .concat();
    sqlx::query(
        r"INSERT INTO agent_room.device (
               id, principal_id, label, platform, public_signing_key, trust_state, created_at
           ) VALUES ($1, $2, '网络 Agent', 'network', $3, 'pending', statement_timestamp())",
    )
    .bind(device)
    .bind(principal.as_uuid())
    .bind(&digest)
    .execute(pool)
    .await
    .expect("网络 Agent 的设备写入应成功");
    sqlx::query(
        r"INSERT INTO agent_room.network_agent (
               id, principal_id, device_id, agent_id, token_digest, display_name, status,
               source_digest, created_at, last_active_at
           ) VALUES ($1, $2, $3, $4, $5, $6, 'provisioning', $5,
                     statement_timestamp(), statement_timestamp())",
    )
    .bind(Uuid::now_v7())
    .bind(principal.as_uuid())
    .bind(device)
    .bind(agent.as_uuid())
    .bind(&digest)
    .bind(format!("Visitor {}", agent.as_uuid().simple()))
    .execute(pool)
    .await
    .expect("网络 Agent 写入应成功");
}

async fn seed_principal(pool: &PgPool, suffix: &str) -> PrincipalId {
    seed_principal_from(pool, suffix, "https://issuer.test").await
}

async fn seed_principal_from(pool: &PgPool, suffix: &str, issuer: &str) -> PrincipalId {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES (
               $1, $6, $2, $3, $4, 'zh-CN', 'active',
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
    .bind(issuer)
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
