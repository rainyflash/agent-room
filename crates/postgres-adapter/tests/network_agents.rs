//! 网络 Agent 的真实数据库往返：一个事务写入主体、设备与秘密，没停用的不重名，
//! 生效绑定 Agent 与实例，停用后名字空出来，固定窗口计数。

use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        NetworkAgentActivation, NetworkAgentBeginOutcome, NetworkAgentProvisioning,
        NetworkAgentSecretKind, NetworkAgentStore, PrincipalRegistration, RateWindowDecision,
        RateWindowPolicy, SealedSecret, SecretDigest,
    },
};
use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    identity::Principal,
    ids::{AgentId, AgentInstanceId, DeviceId, NetworkAgentId, PrincipalId},
    network_agents::{NETWORK_AGENT_ISSUER, NetworkAgentStatus},
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const HOUR: u64 = 3_600_000;

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
async fn 一个事务写入主体设备与秘密_没停用的不重名_停用后名字空出() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let name = format!("Scout {}", &Uuid::now_v7().simple().to_string()[..8]);
    let live_before = repositories.count_live().await.expect("计数");

    let first = provisioning(&name, time(0));
    assert_eq!(
        repositories.begin(&first).await.expect("写入"),
        NetworkAgentBeginOutcome::Created
    );
    let record = repositories
        .find_by_token(&first.token_digest)
        .await
        .expect("按令牌找")
        .expect("找得到");
    assert_eq!(record.id, first.id);
    assert_eq!(record.status, NetworkAgentStatus::Provisioning);
    assert_eq!(record.display_name, name);
    assert_eq!(record.agent_id, None);
    assert_eq!(
        repositories
            .find_secret(first.id, NetworkAgentSecretKind::InstanceSigningSeed)
            .await
            .expect("读秘密"),
        Some(sealed(2))
    );
    assert_eq!(
        repositories.count_live().await.expect("计数"),
        live_before + 1
    );
    let platform: String =
        sqlx::query_scalar("SELECT platform FROM agent_room.device WHERE id = $1")
            .bind(first.device.id().as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("设备已写入");
    assert_eq!(platform, "network");

    // 不分大小写撞名：整个事务回滚，主体也不留下。
    let second = provisioning(&name.to_uppercase(), time(10));
    assert_eq!(
        repositories.begin(&second).await.expect("撞名不算错误"),
        NetworkAgentBeginOutcome::NameTaken
    );
    let leftover: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.principal WHERE id = $1")
            .bind(second.principal.principal.id().as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("查主体");
    assert_eq!(leftover, 0);

    // 停用后令牌不再对应生效的 Agent，名字也空出来。
    repositories
        .disable(first.id, time(20))
        .await
        .expect("停用");
    repositories
        .disable(first.id, time(30))
        .await
        .expect("再停用也行");
    assert_eq!(
        repositories
            .find_by_token(&first.token_digest)
            .await
            .expect("按令牌找")
            .map(|record| record.status),
        Some(NetworkAgentStatus::Disabled)
    );
    let third = provisioning(&name.to_uppercase(), time(40));
    assert_eq!(
        repositories.begin(&third).await.expect("写入"),
        NetworkAgentBeginOutcome::Created
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 生效绑定_agent_与实例_重试同一组也算成功_换一组不行() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let name = format!("Pilot {}", &Uuid::now_v7().simple().to_string()[..8]);
    let provisioning = provisioning(&name, time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let (agent, instance) = seed_agent_instance(
        &database.runtime,
        provisioning.principal.principal.id(),
        provisioning.device.id(),
    )
    .await;
    let activation = NetworkAgentActivation {
        id: provisioning.id,
        agent_id: agent,
        agent_instance_id: instance,
        matrix_access_token: sealed(3),
        activated_at: time(100),
    };
    repositories.activate(&activation).await.expect("生效");
    repositories
        .activate(&activation)
        .await
        .expect("重试同一组");
    let record = repositories
        .find_by_token(&provisioning.token_digest)
        .await
        .expect("按令牌找")
        .expect("找得到");
    assert_eq!(record.status, NetworkAgentStatus::Active);
    assert_eq!(record.agent_id, Some(agent));
    assert_eq!(record.agent_instance_id, Some(instance));
    assert_eq!(record.last_active_at, time(100));
    assert_eq!(
        repositories
            .find_secret(provisioning.id, NetworkAgentSecretKind::MatrixAccessToken)
            .await
            .expect("读秘密"),
        Some(sealed(3))
    );
    let (_, other_instance) = seed_agent_instance(
        &database.runtime,
        provisioning.principal.principal.id(),
        provisioning.device.id(),
    )
    .await;
    let conflict = repositories
        .activate(&NetworkAgentActivation {
            agent_instance_id: other_instance,
            ..activation.clone()
        })
        .await
        .expect_err("生效后不能换实例");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);
    repositories
        .record_activity(provisioning.id, time(50))
        .await
        .expect("更早的活动时间不回退");
    repositories
        .record_activity(provisioning.id, time(500))
        .await
        .expect("记录活动");
    assert_eq!(
        repositories
            .find_by_token(&provisioning.token_digest)
            .await
            .expect("按令牌找")
            .expect("找得到")
            .last_active_at,
        time(500)
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 固定窗口到上限拒绝_窗口过后重新计数() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let bucket = format!("create:hour:{}", Uuid::now_v7().simple());
    let policy = RateWindowPolicy {
        window: DurationMillis::new(HOUR).expect("一小时"),
        limit: 2,
    };
    for offset in [0, 60_000] {
        assert_eq!(
            repositories
                .take(&bucket, time(offset), policy)
                .await
                .expect("计数"),
            RateWindowDecision::Allowed
        );
    }
    let limited = repositories
        .take(&bucket, time(120_000), policy)
        .await
        .expect("计数");
    let hour = i64::try_from(HOUR).expect("一小时");
    assert_eq!(
        limited,
        RateWindowDecision::Limited {
            retry_at: time(hour)
        }
    );
    // 被拒的那次不计数；窗口过去后从这一次重新数。
    assert_eq!(
        repositories
            .take(&bucket, time(hour), policy)
            .await
            .expect("计数"),
        RateWindowDecision::Allowed
    );
    assert_eq!(
        repositories
            .take(&bucket, time(hour + 1), policy)
            .await
            .expect("计数"),
        RateWindowDecision::Allowed
    );
    assert!(matches!(
        repositories
            .take(&bucket, time(hour + 2), policy)
            .await
            .expect("计数"),
        RateWindowDecision::Limited { .. }
    ));
    database.close().await;
}

fn provisioning(name: &str, at: UtcMillis) -> NetworkAgentProvisioning {
    let id = NetworkAgentId::from_uuid(Uuid::now_v7());
    let principal = PrincipalId::from_uuid(Uuid::now_v7());
    let mut device = Device::register(
        DeviceId::from_uuid(Uuid::now_v7()),
        principal,
        "Agent Room 网络 Agent".to_owned(),
        DevicePlatform::Network,
        DevicePublicSigningKey::new(random_bytes().to_vec()).expect("公钥有效"),
        at,
    )
    .expect("设备有效");
    device.verify().expect("可验证");
    NetworkAgentProvisioning {
        id,
        principal: PrincipalRegistration {
            principal: Principal::new(principal),
            oidc_issuer: NETWORK_AGENT_ISSUER.to_owned(),
            oidc_subject: id.to_string(),
            matrix_user_id: format!("@user-{}:matrix.test", id.as_uuid().simple()),
            display_name: name.to_owned(),
            avatar_content_id: None,
            locale: "en".to_owned(),
            registered_at: at,
        },
        device,
        device_label: "Agent Room 网络 Agent".to_owned(),
        token_digest: SecretDigest::from_array(random_bytes()),
        display_name: name.to_owned(),
        source_digest: random_bytes(),
        secrets: vec![
            (NetworkAgentSecretKind::DeviceSigningSeed, sealed(1)),
            (NetworkAgentSecretKind::InstanceSigningSeed, sealed(2)),
        ],
        created_at: at,
    }
}

fn sealed(tag: u8) -> SealedSecret {
    SealedSecret {
        key_version: 1,
        bytes: vec![tag; 60],
    }
}

fn random_bytes() -> [u8; 32] {
    let mut bytes = [0; 32];
    // 两个 UUIDv7 的随机部分足够让测试里的公钥与摘要互不相同。
    bytes[..16].copy_from_slice(Uuid::now_v7().as_bytes());
    bytes[16..].copy_from_slice(Uuid::now_v7().as_bytes());
    bytes
}

/// 网络 Agent 生效前要有它自己的 Agent 与实例；实例挂在网络设备上。
async fn seed_agent_instance(
    pool: &PgPool,
    owner: PrincipalId,
    device: DeviceId,
) -> (AgentId, AgentInstanceId) {
    let agent = AgentId::from_uuid(Uuid::now_v7());
    let binding = Uuid::now_v7();
    let instance = AgentInstanceId::from_uuid(Uuid::now_v7());
    let suffix = agent.as_uuid().simple().to_string();
    let mut transaction = pool.begin().await.expect("事务");
    sqlx::query(
        r"INSERT INTO agent_room.agent (
              id, matrix_user_id, slug, display_name, visibility, lifecycle_state,
              created_at, updated_at
          ) VALUES (
              $1, $2, $3, '网络 Agent', 'private', 'active',
              to_timestamp(1800000000), to_timestamp(1800000000)
          )",
    )
    .bind(agent.as_uuid())
    .bind(format!("@_agent_{suffix}:matrix.test"))
    .bind(format!("host-{suffix}"))
    .execute(&mut *transaction)
    .await
    .expect("Agent 应创建");
    sqlx::query(
        "INSERT INTO agent_room.agent_ownership \
         (principal_id, agent_id, role, granted_by, created_at) \
         VALUES ($1, $2, 'owner', $1, to_timestamp(1800000000))",
    )
    .bind(owner.as_uuid())
    .bind(agent.as_uuid())
    .execute(&mut *transaction)
    .await
    .expect("所有者应写入");
    sqlx::query(
        r"INSERT INTO agent_room.adapter_binding (
              id, agent_id, adapter_type, external_subject_hash,
              capability_version, state, created_at, updated_at
          ) VALUES (
              $1, $2, 'network', NULL, '1.0', 'active',
              to_timestamp(1800000000), to_timestamp(1800000000)
          )",
    )
    .bind(binding)
    .bind(agent.as_uuid())
    .execute(&mut *transaction)
    .await
    .expect("适配器绑定应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance (
              id, agent_id, device_id, adapter_binding_id, public_signing_key,
              matrix_device_id, status, created_at
          ) VALUES (
              $1, $2, $3, $4, $5, $6, 'connecting', to_timestamp(1800000000)
          )",
    )
    .bind(instance.as_uuid())
    .bind(agent.as_uuid())
    .bind(device.as_uuid())
    .bind(binding)
    .bind(random_bytes().as_slice())
    .bind(format!("AR_{}", instance.as_uuid().simple()))
    .execute(&mut *transaction)
    .await
    .expect("Agent 实例应创建");
    transaction.commit().await.expect("提交");
    (agent, instance)
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
