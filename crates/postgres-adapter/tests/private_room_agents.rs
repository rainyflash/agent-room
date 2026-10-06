//! 私人房间 Agent 口令的真实数据库往返：口令只存摘要且一房一个、Agent 成员的加入与移出、猜错窗口，
//! 以及网络 Agent 拿房间号敲门（`specs/network-agents/knock.md`）。

use std::env;

use agent_room_application::{
    persistence::{RepositoryErrorKind, RepositoryResult},
    ports::{
        JoinCodeAttemptPolicy, PrivateRoomAgentAccessStore, PrivateRoomAgentKnockOutcome,
        PrivateRoomAgentKnockRecord, PrivateRoomJoinCodeRecord, PrivateRoomSnapshot,
        PrivateRoomStore, SecretDigest,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    join_codes::{
        PrivateRoomAgentJoinedVia, PrivateRoomAgentKnockStatus, PrivateRoomAgentMemberStatus,
    },
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const HOUR: i64 = 3_600_000;

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
async fn 口令只存摘要且一房一个_按摘要找回_停用后找不到() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "code-owner").await;
    let catalog_id = seed_room(&repositories, owner).await;
    let first = digest(1);
    let record = code_record(catalog_id, owner, time(0));
    repositories
        .replace_join_code(&record, &first)
        .await
        .expect("写入口令");
    assert_eq!(
        repositories.join_code(catalog_id).await.expect("读取"),
        Some(record.clone())
    );
    assert_eq!(
        repositories.find_join_code(&first).await.expect("按摘要找"),
        Some(record)
    );
    // 换口令：旧摘要失效。
    let second = digest(2);
    let newer = code_record(catalog_id, owner, time(10));
    repositories
        .replace_join_code(&newer, &second)
        .await
        .expect("更换口令");
    assert_eq!(
        repositories.find_join_code(&first).await.expect("查找"),
        None
    );
    assert_eq!(
        repositories.find_join_code(&second).await.expect("查找"),
        Some(newer)
    );
    assert!(
        repositories
            .clear_join_code(catalog_id)
            .await
            .expect("停用")
    );
    assert!(
        !repositories
            .clear_join_code(catalog_id)
            .await
            .expect("再停用")
    );
    assert_eq!(
        repositories.find_join_code(&second).await.expect("查找"),
        None
    );
    assert_eq!(
        repositories.join_code(catalog_id).await.expect("读取"),
        None
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_成员加入_移出_再加入都能往返_名字取自_agent_与主人() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "room-owner").await;
    let catalog_id = seed_room(&repositories, owner).await;
    let agent_owner = seed_principal(&database.runtime, "agent-owner").await;
    let agent = seed_agent(&database.runtime, agent_owner).await;

    repositories
        .admit_agent(
            catalog_id,
            agent,
            PrivateRoomPermissions::AGENT_MEMBER,
            PrivateRoomAgentJoinedVia::Code,
            time(0),
        )
        .await
        .expect("加入");
    // 重复加入保持原来的状态时间。
    repositories
        .admit_agent(
            catalog_id,
            agent,
            PrivateRoomPermissions::AGENT_MEMBER,
            PrivateRoomAgentJoinedVia::Knock,
            time(5),
        )
        .await
        .expect("重复加入");
    let joined = repositories
        .agent_member(catalog_id, agent)
        .await
        .expect("读取")
        .expect("已加入");
    assert_eq!(joined.status, PrivateRoomAgentMemberStatus::Joined);
    assert_eq!(joined.permissions, PrivateRoomPermissions::AGENT_MEMBER);
    assert_eq!(joined.display_name, "口令测试 Agent");
    assert_eq!(
        joined.owner_display_name.as_deref(),
        Some("测试主体 agent-owner")
    );
    assert_eq!(joined.joined_at, time(0));
    assert_eq!(joined.status_changed_at, time(0));
    assert_eq!(
        joined_via(&database.runtime, catalog_id, agent).await,
        "code",
        "已经在里面的再加入不改进法"
    );

    assert!(
        repositories
            .remove_agent(catalog_id, agent, time(20))
            .await
            .expect("移出")
    );
    assert!(
        !repositories
            .remove_agent(catalog_id, agent, time(21))
            .await
            .expect("再移出")
    );
    let removed = repositories
        .agent_member(catalog_id, agent)
        .await
        .expect("读取")
        .expect("仍有记录");
    assert_eq!(removed.status, PrivateRoomAgentMemberStatus::Removed);
    assert_eq!(removed.permissions, PrivateRoomPermissions::NONE);
    assert_eq!(removed.status_changed_at, time(20));

    repositories
        .admit_agent(
            catalog_id,
            agent,
            PrivateRoomPermissions::AGENT_MEMBER,
            PrivateRoomAgentJoinedVia::Knock,
            time(30),
        )
        .await
        .expect("再加入");
    let members = repositories.agent_members(catalog_id).await.expect("列出");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].status, PrivateRoomAgentMemberStatus::Joined);
    assert_eq!(members[0].status_changed_at, time(30));
    assert_eq!(members[0].joined_at, time(0));
    assert_eq!(
        joined_via(&database.runtime, catalog_id, agent).await,
        "knock",
        "移出后再进来按这一次的进法记"
    );
    assert!(
        repositories
            .agent_member(catalog_id, AgentId::from_uuid(Uuid::now_v7()))
            .await
            .expect("读取")
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 猜错按固定窗口计数_到上限后等窗口结束_窗口过后重新计() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let caller = format!("device:{}", Uuid::now_v7());
    let policy = JoinCodeAttemptPolicy {
        window: DurationMillis::new(u64::try_from(HOUR).unwrap()).unwrap(),
        max_failures: 2,
    };
    assert_eq!(
        repositories
            .join_code_retry_at(&caller, time(0), policy)
            .await
            .expect("查询"),
        None
    );
    repositories
        .record_join_code_failure(&caller, time(0), policy)
        .await
        .expect("记一次");
    assert_eq!(
        repositories
            .join_code_retry_at(&caller, time(1), policy)
            .await
            .expect("查询"),
        None
    );
    repositories
        .record_join_code_failure(&caller, time(2), policy)
        .await
        .expect("记两次");
    assert_eq!(
        repositories
            .join_code_retry_at(&caller, time(3), policy)
            .await
            .expect("查询"),
        Some(time(HOUR))
    );
    // 窗口过后自然放开，再猜错从头计。
    assert_eq!(
        repositories
            .join_code_retry_at(&caller, time(HOUR), policy)
            .await
            .expect("查询"),
        None
    );
    repositories
        .record_join_code_failure(&caller, time(HOUR + 1), policy)
        .await
        .expect("新窗口");
    assert_eq!(
        repositories
            .join_code_retry_at(&caller, time(HOUR + 2), policy)
            .await
            .expect("查询"),
        None
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 敲门记一条_再敲从现在重新算_作废的不算在等() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "knock-owner").await;
    let catalog_id = seed_room(&repositories, owner).await;
    let agent_owner = seed_principal(&database.runtime, "knock-agent-owner").await;
    let agent = seed_agent(&database.runtime, agent_owner).await;

    let first = waiting(
        repositories
            .knock(catalog_id, agent, time(0), time(HOUR), 5)
            .await,
    );
    assert_eq!(first.catalog_id, catalog_id);
    assert_eq!(first.agent_id, agent);
    assert_eq!(first.display_name, "口令测试 Agent");
    assert_eq!(first.knocked_at, time(0));
    assert_eq!(first.expires_at, time(HOUR));
    assert_eq!(first.decided_at, None);
    let again = waiting(
        repositories
            .knock(catalog_id, agent, time(10), time(10 + HOUR), 5)
            .await,
    );
    assert_eq!(again.knocked_at, time(10), "再敲从现在重新算");
    assert_eq!(again.expires_at, time(10 + HOUR));
    assert_eq!(
        waiting_agents(&repositories, catalog_id, time(20)).await,
        [agent]
    );
    assert!(
        waiting_agents(&repositories, catalog_id, time(10 + HOUR))
            .await
            .is_empty(),
        "作废的不算在等"
    );
    assert_eq!(
        repositories
            .agent_knocks(agent, time(0))
            .await
            .expect("Agent 敲过的门")
            .len(),
        1,
        "同一个 Agent 对同一个房间只记一条"
    );
    assert!(
        repositories
            .agent_knocks(agent, time(11))
            .await
            .expect("Agent 敲过的门")
            .is_empty()
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 放行后记为敲门进来的_不再算在等_被移出以后可以再敲() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "admit-owner").await;
    let catalog_id = seed_room(&repositories, owner).await;
    let agent_owner = seed_principal(&database.runtime, "admit-agent-owner").await;
    let agent = seed_agent(&database.runtime, agent_owner).await;
    waiting(
        repositories
            .knock(catalog_id, agent, time(0), time(HOUR), 5)
            .await,
    );

    repositories
        .admit_agent(
            catalog_id,
            agent,
            PrivateRoomPermissions::AGENT_MEMBER,
            PrivateRoomAgentJoinedVia::Knock,
            time(30),
        )
        .await
        .expect("放行");
    assert_eq!(
        joined_via(&database.runtime, catalog_id, agent).await,
        "knock"
    );
    assert!(
        repositories
            .decide_knock(
                catalog_id,
                agent,
                PrivateRoomAgentKnockStatus::Admitted,
                owner,
                time(31)
            )
            .await
            .expect("记为放进来了")
    );
    assert!(
        waiting_agents(&repositories, catalog_id, time(32))
            .await
            .is_empty()
    );
    let decided = repositories
        .agent_knocks(agent, time(0))
        .await
        .expect("Agent 敲过的门");
    assert_eq!(decided[0].status, PrivateRoomAgentKnockStatus::Admitted);
    assert_eq!(decided[0].decided_at, Some(time(31)));

    // 被移出以后可以再敲，由管理者再决定。
    repositories
        .remove_agent(catalog_id, agent, time(40))
        .await
        .expect("移出");
    let after_removal = waiting(
        repositories
            .knock(catalog_id, agent, time(50), time(50 + HOUR), 5)
            .await,
    );
    assert_eq!(after_removal.decided_at, None);
    assert_eq!(
        waiting_agents(&repositories, catalog_id, time(51)).await,
        [agent]
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 房间满了按最早作废的时间限流_停用的不算在等也不占位置() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "full-owner").await;
    let catalog_id = seed_room(&repositories, owner).await;
    let agent_owner = seed_principal(&database.runtime, "full-agent-owner").await;
    let first = seed_agent(&database.runtime, agent_owner).await;
    let second = seed_agent(&database.runtime, agent_owner).await;
    let third = seed_agent(&database.runtime, agent_owner).await;

    waiting(
        repositories
            .knock(catalog_id, first, time(0), time(HOUR), 2)
            .await,
    );
    waiting(
        repositories
            .knock(catalog_id, second, time(10), time(10 + HOUR), 2)
            .await,
    );
    assert_eq!(
        repositories
            .knock(catalog_id, third, time(20), time(20 + HOUR), 2)
            .await
            .expect("满了也是正常回答"),
        PrivateRoomAgentKnockOutcome::RoomFull {
            retry_at: time(HOUR)
        }
    );
    // 已经在等的再敲不算多占一个。
    waiting(
        repositories
            .knock(catalog_id, first, time(30), time(30 + HOUR), 2)
            .await,
    );

    sqlx::query("UPDATE agent_room.agent SET lifecycle_state = 'suspended' WHERE id = $1")
        .bind(first.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("停用 Agent");
    waiting(
        repositories
            .knock(catalog_id, third, time(70), time(70 + HOUR), 2)
            .await,
    );
    assert_eq!(
        waiting_agents(&repositories, catalog_id, time(80)).await,
        [second, third]
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 没让进的再敲还是没让进_回答只能是放进来了或没让进_没有这个房间() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "decline-owner").await;
    let catalog_id = seed_room(&repositories, owner).await;
    let agent_owner = seed_principal(&database.runtime, "decline-agent-owner").await;
    let agent = seed_agent(&database.runtime, agent_owner).await;
    waiting(
        repositories
            .knock(catalog_id, agent, time(10), time(10 + HOUR), 2)
            .await,
    );

    assert!(
        repositories
            .decide_knock(
                catalog_id,
                agent,
                PrivateRoomAgentKnockStatus::Declined,
                owner,
                time(40)
            )
            .await
            .expect("不让进")
    );
    assert!(
        !repositories
            .decide_knock(
                catalog_id,
                agent,
                PrivateRoomAgentKnockStatus::Declined,
                owner,
                time(41)
            )
            .await
            .expect("原本不在等")
    );
    let PrivateRoomAgentKnockOutcome::Declined(declined) = repositories
        .knock(catalog_id, agent, time(50), time(50 + HOUR), 2)
        .await
        .expect("再敲")
    else {
        panic!("没让进的再敲还是没让进");
    };
    assert_eq!(declined.decided_at, Some(time(40)));
    assert_eq!(declined.knocked_at, time(10), "不刷新");
    assert!(
        repositories
            .decide_knock(
                catalog_id,
                agent,
                PrivateRoomAgentKnockStatus::Waiting,
                owner,
                time(60)
            )
            .await
            .is_err(),
        "回答只能是放进来了或没让进"
    );

    let unknown = repositories
        .knock(
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            agent,
            time(90),
            time(90 + HOUR),
            2,
        )
        .await
        .expect_err("没有这个房间");
    assert_eq!(unknown.kind(), RepositoryErrorKind::NotFound);
    database.close().await;
}

fn waiting(outcome: RepositoryResult<PrivateRoomAgentKnockOutcome>) -> PrivateRoomAgentKnockRecord {
    match outcome.expect("敲门") {
        PrivateRoomAgentKnockOutcome::Waiting(record) => {
            assert_eq!(record.status, PrivateRoomAgentKnockStatus::Waiting);
            record
        }
        other => panic!("应在等：{other:?}"),
    }
}

async fn waiting_agents(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    now: UtcMillis,
) -> Vec<AgentId> {
    repositories
        .waiting_knocks(catalog_id, now)
        .await
        .expect("在等的敲门")
        .into_iter()
        .map(|knock| knock.agent_id)
        .collect()
}

async fn joined_via(pool: &PgPool, catalog_id: RoomCatalogId, agent: AgentId) -> String {
    sqlx::query_scalar(
        "SELECT joined_via FROM agent_room.private_room_agent_member
          WHERE catalog_entry_id = $1 AND agent_id = $2",
    )
    .bind(catalog_id.as_uuid())
    .bind(agent.as_uuid())
    .fetch_one(pool)
    .await
    .expect("读进法")
}

fn code_record(
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    created_at: UtcMillis,
) -> PrivateRoomJoinCodeRecord {
    PrivateRoomJoinCodeRecord {
        catalog_id,
        permissions: PrivateRoomPermissions::AGENT_MEMBER,
        created_by: owner,
        created_at,
    }
}

/// 测试里的摘要只需要彼此不同、32 字节，并在同一次运行里与别的测试不冲突。
fn digest(seed: u8) -> SecretDigest {
    let mut bytes = [seed; 32];
    bytes[..16].copy_from_slice(Uuid::now_v7().as_bytes());
    SecretDigest::from_array(bytes)
}

async fn seed_room(repositories: &PostgresRepositories, owner: PrincipalId) -> RoomCatalogId {
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(repositories, &snapshot(catalog_id, owner), time(0))
        .await
        .expect("创建房间");
    catalog_id
}

fn snapshot(catalog_id: RoomCatalogId, owner: PrincipalId) -> PrivateRoomSnapshot {
    let catalog = RoomCatalog::new(
        catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "口令项目室".to_owned(),
            description: String::new(),
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
                "!code{}:matrix.test",
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

/// 与现有测试相同的做法：先以暂停状态建 Agent，建好所有权再激活，满足“有效 Agent 必须有主人”。
async fn seed_agent(pool: &PgPool, owner: PrincipalId) -> AgentId {
    let agent = AgentId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.agent (
               id, matrix_user_id, slug, display_name, description, visibility,
               lifecycle_state, created_at, updated_at, version
            ) VALUES ($1, $2, $3, '口令测试 Agent', '', 'private',
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
