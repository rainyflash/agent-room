use std::{
    env,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_room_application::{
    ports::{
        AgentLobbyAccessRepository, AgentRoomKind, AgentRoomPlacementStore, Clock,
        DeviceRevocationTransaction, DeviceSecurityEvent, MatrixCreateRoom, MatrixEventId,
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomAliasLocalpart,
        MatrixRoomId, MatrixRoomKind, ModerationEffectGateway, ModerationEffectTarget, PortFuture,
        RevokedAgentDepartureAudit, RevokedAgentDepartureAuditOutcome, RevokedAgentDepartureClaim,
        RevokedAgentDepartureFacts, RevokedAgentDepartureStore, RoomAgentKind,
        RoomAllocationEvidence, RoomAllocationMode, RoomAllocationStore, RoomMembershipGateway,
        RoomPeopleDirectory, RoomProvisioningGateway,
    },
    rooms::{
        EnterLobbyDependencies, EnterLobbyOutcome, EnterLobbyService, JoinLobbyDependencies,
        JoinLobbyRequest, JoinLobbyService, LobbyJoinKind, LobbyJoinPolicy,
        LobbyProvisioningDependencies, LobbyProvisioningIdentifierFactory, LobbyProvisioningPolicy,
        LobbyProvisioningService, RoomReservationIdentifierFactory,
    },
};
use agent_room_domain::{
    ids::{
        AgentId, AgentInstanceId, DeviceId, OutboxEventId, PrincipalId, RoomCatalogId,
        RoomInstanceId, RoomProvisioningJobId, RoomProvisioningLeaseId, RoomReservationId,
    },
    moderation::ModerationAction,
    rooms::{MatrixRoomReference, RoomCatalogStatus, RoomReservationState},
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
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

#[derive(Debug)]
struct 测试运行时;

impl Clock for 测试运行时 {
    fn now(&self) -> UtcMillis {
        test_time(1_000)
    }
}

impl RoomReservationIdentifierFactory for 测试运行时 {
    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }
}

impl LobbyProvisioningIdentifierFactory for 测试运行时 {
    fn room_provisioning_job_id(&self) -> RoomProvisioningJobId {
        RoomProvisioningJobId::from_uuid(Uuid::now_v7())
    }

    fn room_provisioning_lease_id(&self) -> RoomProvisioningLeaseId {
        RoomProvisioningLeaseId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatrixCall {
    CreateSpace,
    CreateInstance,
    Attach,
    /// 新分片开始接人之前补的禁言、封禁。
    CarryOverModeration,
    Join,
    Leave,
}

struct 测试Matrix {
    calls: Mutex<Vec<MatrixCall>>,
    fail_join: AtomicBool,
    space_id: String,
    instance_id: String,
}

impl 测试Matrix {
    fn new(fail_join: bool) -> Self {
        let suffix = Uuid::now_v7().simple().to_string();
        Self {
            calls: Mutex::new(Vec::new()),
            fail_join: AtomicBool::new(fail_join),
            space_id: format!("!space-{suffix}:matrix.test"),
            instance_id: format!("!instance-{suffix}:matrix.test"),
        }
    }

    fn calls(&self) -> Vec<MatrixCall> {
        self.calls.lock().expect("Matrix 调用锁可用").clone()
    }
}

impl RoomProvisioningGateway for 测试Matrix {
    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            let (call, room_id) = match request.kind() {
                MatrixRoomKind::Space => (MatrixCall::CreateSpace, &self.space_id),
                MatrixRoomKind::Conversation => (MatrixCall::CreateInstance, &self.instance_id),
            };
            self.calls.lock().expect("Matrix 调用锁可用").push(call);
            MatrixRoomId::new(room_id.clone()).map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::CreateRoom,
                    MatrixFailureKind::InvalidResponse,
                )
            })
        })
    }

    fn resolve_room_alias<'a>(
        &'a self,
        _alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async {
            Err(MatrixFailure::new(
                MatrixOperation::ResolveRoomAlias,
                MatrixFailureKind::NotFound,
            ))
        })
    }

    fn attach_child<'a>(
        &'a self,
        _space_id: &'a MatrixRoomId,
        _child_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::Attach);
            MatrixEventId::new("$attach:matrix.test").map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::SendStateEvent,
                    MatrixFailureKind::InvalidResponse,
                )
            })
        })
    }
}

impl ModerationEffectGateway for 测试Matrix {
    fn apply<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::CarryOverModeration);
            Ok(())
        })
    }

    fn reverse<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        panic!("建分片只补生效的治理，不撤销")
    }

    fn contains_event<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<bool>> {
        panic!("建分片用不着找消息")
    }
}

impl RoomMembershipGateway for 测试Matrix {
    fn join<'a>(&'a self, _room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::Join);
            if self.fail_join.load(Ordering::SeqCst) {
                Err(MatrixFailure::new(
                    MatrixOperation::Join,
                    MatrixFailureKind::DependencyUnavailable,
                ))
            } else {
                Ok(())
            }
        })
    }

    fn leave<'a>(&'a self, _room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::Leave);
            Ok(())
        })
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 空目录可在一次用例中自动建房预约并确认加入() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let matrix = Arc::new(测试Matrix::new(false));
    let service = service(database.runtime.clone(), matrix.clone());

    let outcome = service
        .enter(request(&fixture))
        .await
        .expect("空目录应自动供给后加入");

    assert!(matches!(
        outcome,
        EnterLobbyOutcome::Joined {
            kind: LobbyJoinKind::NewAssignment,
            ..
        }
    ));
    assert_eq!(
        matrix.calls(),
        vec![
            MatrixCall::CreateSpace,
            MatrixCall::CreateInstance,
            MatrixCall::Attach,
            MatrixCall::Join,
        ]
    );
    assert_eq!(
        catalog_space(&database.runtime, fixture.catalog).await,
        Some(matrix.space_id.clone())
    );
    assert_eq!(room_count(&database.runtime, fixture.catalog).await, 1);
    assert_eq!(
        committed_reservation_count(&database.runtime, fixture.instance).await,
        1
    );
    assert_eq!(allocated_slots(&database.runtime, fixture.catalog).await, 1);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn matrix_加入失败会释放新房槽位且绝不留下已加入预约() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let matrix = Arc::new(测试Matrix::new(true));
    let service = service(database.runtime.clone(), matrix.clone());

    service
        .enter(request(&fixture))
        .await
        .expect_err("Matrix 加入失败必须终止用例");

    assert_eq!(matrix.calls().last(), Some(&MatrixCall::Join));
    assert_eq!(
        committed_reservation_count(&database.runtime, fixture.instance).await,
        0
    );
    assert_eq!(
        released_reservation_count(&database.runtime, fixture.instance).await,
        1
    );
    assert_eq!(allocated_slots(&database.runtime, fixture.catalog).await, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 大厅访问记录绑定实例设备和权威_matrix_身份并反映撤销() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());

    let active = repositories
        .find_lobby_access(fixture.instance)
        .await
        .expect("访问记录应可查询")
        .expect("实例应存在");
    assert_eq!(active.agent_id, fixture.agent);
    assert_eq!(active.agent_instance_id, fixture.instance);
    assert_eq!(active.device_id, fixture.device);
    assert!(active.active);
    // 按 Agent 找（主人带进房间用）得到同一份。
    assert_eq!(
        repositories
            .lobby_accesses_of_agent(fixture.agent)
            .await
            .expect("按 Agent 找"),
        [active]
    );

    sqlx::query(
        "UPDATE agent_room.agent_instance SET status = 'revoked', revoked_at = now() WHERE id = $1",
    )
    .bind(fixture.instance.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("实例应可撤销");
    let revoked = repositories
        .find_lobby_access(fixture.instance)
        .await
        .expect("撤销后仍应可查询")
        .expect("撤销不会删除实例");
    assert!(!revoked.active);
    // 吊销了的连接不再列出。
    assert!(
        repositories
            .lobby_accesses_of_agent(fixture.agent)
            .await
            .expect("按 Agent 找")
            .is_empty()
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_在哪些房间按已确认的位置算_放掉以后就不在了() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let matrix = Arc::new(测试Matrix::new(false));
    let repositories = PostgresRepositories::new(database.runtime.clone());
    assert!(
        repositories
            .placements(fixture.agent)
            .await
            .expect("还没进房间时可查询")
            .is_empty()
    );
    service(database.runtime.clone(), matrix.clone())
        .enter(request(&fixture))
        .await
        .expect("进大厅");

    let placements = repositories
        .placements(fixture.agent)
        .await
        .expect("进房间以后可查询");
    assert_eq!(placements.len(), 1);
    let placement = &placements[0];
    assert_eq!(placement.agent_instance_id, fixture.instance);
    assert_eq!(placement.catalog_id, fixture.catalog);
    assert_eq!(placement.matrix_room_id.as_str(), matrix.instance_id);
    assert_eq!(placement.kind, AgentRoomKind::PublicLobby);
    assert_eq!(placement.name, "纵向测试大厅");
    assert_eq!(placement.status, RoomCatalogStatus::Active);
    let user = repositories
        .matrix_user(fixture.agent)
        .await
        .expect("可查询")
        .expect("Agent 有 Matrix 账号");
    assert!(user.as_str().starts_with("@agent-"));
    assert!(
        repositories
            .matrix_user(AgentId::from_uuid(Uuid::now_v7()))
            .await
            .expect("可查询")
            .is_none()
    );

    RoomAllocationStore::transition(
        &repositories,
        placement.reservation_id,
        RoomReservationState::Committed,
        RoomReservationState::Released,
        test_time(2_000),
    )
    .await
    .expect("放掉位置");
    assert!(
        repositories
            .placements(fixture.agent)
            .await
            .expect("放掉以后可查询")
            .is_empty()
    );
    assert_eq!(allocated_slots(&database.runtime, fixture.catalog).await, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 大厅里的本机_agent_写主人的名字_不在的不回答() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let absent = AgentId::from_uuid(Uuid::now_v7());
    assert!(
        repositories
            .agent_owners(fixture.catalog, &[fixture.agent])
            .await
            .expect("还没进大厅时可查询")
            .is_empty()
    );
    service(database.runtime.clone(), Arc::new(测试Matrix::new(false)))
        .enter(request(&fixture))
        .await
        .expect("进大厅");

    let owners = repositories
        .agent_owners(fixture.catalog, &[fixture.agent, absent])
        .await
        .expect("可查询");

    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].agent_id, fixture.agent);
    assert_eq!(owners[0].kind, RoomAgentKind::Local);
    let owner = owners[0].owner.as_ref().expect("本机 Agent 有主人");
    assert_eq!(owner.display_name, "纵向测试主体");
    assert_eq!(owners[0].admitted_by, None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 撤销电脑时记下它的_agent_要离开的房间_一条一条领() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let matrix = Arc::new(测试Matrix::new(false));
    service(database.runtime.clone(), matrix.clone())
        .enter(request(&fixture))
        .await
        .expect("进大厅");
    let repositories = PostgresRepositories::new(database.runtime.clone());
    revoke_device(&repositories, &database.runtime, fixture.device).await;

    let claim = repositories
        .claim_departure(Some(fixture.device), test_time(1_000), test_time(121_000))
        .await
        .expect("可领")
        .expect("撤销时记下了它在的大厅");
    assert_eq!(claim.agent_id, fixture.agent);
    assert_eq!(claim.device_id, fixture.device);
    assert_eq!(claim.attempt, 1);
    assert_eq!(claim.previous_failure_code, None);
    assert!(
        repositories
            .claim_departure(Some(fixture.device), test_time(1_000), test_time(121_000))
            .await
            .expect("可领")
            .is_none(),
        "领着的别人领不到"
    );
    let facts = repositories
        .departure_facts(&claim)
        .await
        .expect("可查询")
        .expect("本机 Agent 在公开大厅");
    assert!(!facts.live_connection_remains);
    assert_eq!(facts.kind, AgentRoomKind::PublicLobby);
    assert_eq!(facts.catalog_id, fixture.catalog);
    assert_eq!(facts.matrix_room_id.as_str(), matrix.instance_id);
    assert_eq!(facts.revoked_reservations.len(), 1);

    finish_departure_after_one_retry(&repositories, &database.runtime, claim, facts).await;
    assert_eq!(allocated_slots(&database.runtime, fixture.catalog).await, 0);
    assert_eq!(
        departure_audits(&database.runtime, fixture.agent).await,
        [
            ("agent.room.depart_failed".to_owned(), "failed".to_owned()),
            ("agent.room.departed".to_owned(), "allowed".to_owned()),
        ]
    );

    revoke_device(&repositories, &database.runtime, fixture.device).await;
    assert!(
        repositories
            .claim_departure(
                Some(fixture.device),
                test_time(1_000_000),
                test_time(1_120_000)
            )
            .await
            .expect("可领")
            .is_none(),
        "位置已经放掉了，再点一次撤销不再记"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 同一个房间里别的电脑上还有它的连接时只放撤销的那个位置() {
    let database = TestDatabase::connect().await;
    let fixture = seed_fixture(&database.runtime).await;
    let matrix = Arc::new(测试Matrix::new(false));
    let lobby = service(database.runtime.clone(), matrix.clone());
    lobby.enter(request(&fixture)).await.expect("进大厅");
    let second = add_connection_on_another_computer(&database.runtime, &fixture).await;
    lobby
        .enter(JoinLobbyRequest {
            agent_instance_id: second,
            ..request(&fixture)
        })
        .await
        .expect("另一台电脑上的连接也进大厅");
    let repositories = PostgresRepositories::new(database.runtime.clone());
    revoke_device(&repositories, &database.runtime, fixture.device).await;

    let claim = repositories
        .claim_departure(Some(fixture.device), test_time(1_000), test_time(121_000))
        .await
        .expect("可领")
        .expect("撤销时记下了它在的大厅");
    let facts = repositories
        .departure_facts(&claim)
        .await
        .expect("可查询")
        .expect("本机 Agent 在公开大厅");

    assert!(facts.live_connection_remains);
    assert_eq!(facts.revoked_reservations.len(), 1);
    assert_eq!(
        committed_reservation_count(&database.runtime, second).await,
        1
    );

    database.close().await;
}

/// 第一次没做成、排到 30 秒以后；之前领的那次删不掉，再领一次放掉位置、做完删掉。
async fn finish_departure_after_one_retry(
    repositories: &PostgresRepositories,
    pool: &PgPool,
    claim: RevokedAgentDepartureClaim,
    facts: RevokedAgentDepartureFacts,
) {
    let audit = |outcome| RevokedAgentDepartureAudit {
        occurred_at: test_time(1_000),
        catalog_id: facts.catalog_id,
        matrix_room_id: facts.matrix_room_id.clone(),
        outcome,
    };
    let failed = audit(RevokedAgentDepartureAuditOutcome::Failed {
        failure_code: "matrix.unavailable",
    });
    assert!(
        repositories
            .retry_departure(
                &claim,
                test_time(31_000),
                "matrix.unavailable",
                Some(&failed)
            )
            .await
            .expect("可排下次")
    );
    assert!(
        repositories
            .claim_departure(Some(claim.device_id), test_time(30_999), test_time(150_999))
            .await
            .expect("可领")
            .is_none(),
        "没到时间不领"
    );
    let retried = repositories
        .claim_departure(Some(claim.device_id), test_time(31_000), test_time(151_000))
        .await
        .expect("可领")
        .expect("到时间再领");
    assert_eq!(retried.attempt, 2);
    assert_eq!(
        retried.previous_failure_code.as_deref(),
        Some("matrix.unavailable")
    );
    assert!(
        !repositories
            .complete_departure(&claim, None)
            .await
            .expect("可删"),
        "之前领的那次删不掉"
    );
    for reservation in &facts.revoked_reservations {
        RoomAllocationStore::transition(
            repositories,
            *reservation,
            RoomReservationState::Committed,
            RoomReservationState::Released,
            test_time(31_000),
        )
        .await
        .expect("放掉位置");
    }
    let departed = audit(RevokedAgentDepartureAuditOutcome::Departed {
        left_room: true,
        released_reservations: 1,
    });
    assert!(
        repositories
            .complete_departure(&retried, Some(&departed))
            .await
            .expect("可删")
    );
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.revoked_agent_departure WHERE agent_id = $1",
    )
    .bind(claim.agent_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("可统计");
    assert_eq!(remaining, 0);
}

async fn revoke_device(repositories: &PostgresRepositories, pool: &PgPool, device: DeviceId) {
    let principal: Uuid =
        sqlx::query_scalar("SELECT principal_id FROM agent_room.device WHERE id = $1")
            .bind(device.as_uuid())
            .fetch_one(pool)
            .await
            .expect("设备有主人");
    DeviceRevocationTransaction::revoke(
        repositories,
        PrincipalId::from_uuid(principal),
        device,
        DeviceSecurityEvent {
            id: OutboxEventId::from_uuid(Uuid::now_v7()),
            occurred_at: test_time(1_000),
        },
    )
    .await
    .expect("撤销电脑");
}

async fn departure_audits(pool: &PgPool, agent: AgentId) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT action, outcome FROM agent_room.audit_event \
         WHERE target_kind = 'agent' AND target_reference = $1 AND actor_kind = 'service' \
         ORDER BY id",
    )
    .bind(agent.to_string())
    .fetch_all(pool)
    .await
    .expect("可读审计")
}

/// 同一个 Agent 在同一个主人的另一台电脑上的连接。
async fn add_connection_on_another_computer(pool: &PgPool, fixture: &Fixture) -> AgentInstanceId {
    let device_id = Uuid::now_v7();
    let instance_id = Uuid::now_v7();
    sqlx::query(
        r"INSERT INTO agent_room.device (
              id, principal_id, label, platform, public_signing_key,
              trust_state, verified_at, created_at
          )
          SELECT $1, principal_id, '另一台电脑', 'macos', $2, 'verified',
                 to_timestamp(1700000000), to_timestamp(1700000000)
            FROM agent_room.device WHERE id = $3",
    )
    .bind(device_id)
    .bind(test_key(device_id))
    .bind(fixture.device.as_uuid())
    .execute(pool)
    .await
    .expect("另一台电脑应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance (
              id, agent_id, device_id, adapter_binding_id, public_signing_key,
              matrix_device_id, status, created_at
          )
          SELECT $1, agent_id, $2, adapter_binding_id, $3, $4, 'connecting',
                 to_timestamp(1700000000)
            FROM agent_room.agent_instance WHERE id = $5",
    )
    .bind(instance_id)
    .bind(device_id)
    .bind(test_key(instance_id))
    .bind(format!("AR_{}", instance_id.simple()))
    .bind(fixture.instance.as_uuid())
    .execute(pool)
    .await
    .expect("另一台电脑上的连接应创建");
    AgentInstanceId::from_uuid(instance_id)
}

fn service(pool: PgPool, matrix: Arc<测试Matrix>) -> EnterLobbyService {
    let repositories = Arc::new(PostgresRepositories::new(pool));
    let runtime = Arc::new(测试运行时);
    let joins = Arc::new(JoinLobbyService::new(
        JoinLobbyDependencies {
            allocations: repositories.clone(),
            membership: matrix.clone(),
            identifiers: runtime.clone(),
            clock: runtime.clone(),
        },
        LobbyJoinPolicy::new(DurationMillis::new(60_000).expect("预约时长有效"))
            .expect("预约策略有效"),
    ));
    let provisioning = Arc::new(LobbyProvisioningService::new(
        LobbyProvisioningDependencies {
            store: repositories.clone(),
            matrix: matrix.clone(),
            // 真的去库里读要补的治理；这个大厅没人被禁言、封禁，一条也不补。
            moderation: repositories,
            moderation_effects: matrix,
            identifiers: runtime.clone(),
            clock: runtime,
        },
        LobbyProvisioningPolicy::new(DurationMillis::new(30_000).expect("租约时长有效"))
            .expect("供给策略有效"),
    ));
    EnterLobbyService::new(EnterLobbyDependencies {
        joins,
        provisioning,
    })
}

struct Fixture {
    agent: AgentId,
    instance: AgentInstanceId,
    device: DeviceId,
    catalog: RoomCatalogId,
}

async fn seed_fixture(pool: &PgPool) -> Fixture {
    let principal_id = Uuid::now_v7();
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let agent_instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let suffix = agent_id.as_uuid().simple().to_string();
    let mut transaction = pool.begin().await.expect("测试事务应启动");
    let device = insert_identity(
        &mut transaction,
        principal_id,
        agent_id,
        agent_instance_id,
        &suffix,
    )
    .await;
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
              id, kind, slug, name, description, language,
              visibility, status, created_at, updated_at
          ) VALUES (
              $1, 'public_lobby', $2, '纵向测试大厅', '自动供给纵向验收', 'zh-CN',
              'public', 'active', to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(catalog_id.as_uuid())
    .bind(format!("entry-{}", &suffix[..24]))
    .execute(&mut *transaction)
    .await
    .expect("测试目录应创建");
    transaction.commit().await.expect("测试夹具应提交");
    Fixture {
        agent: agent_id,
        instance: agent_instance_id,
        device,
        catalog: catalog_id,
    }
}

async fn insert_identity(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: Uuid,
    agent_id: AgentId,
    agent_instance_id: AgentInstanceId,
    suffix: &str,
) -> DeviceId {
    sqlx::query(
        r"INSERT INTO agent_room.principal (
              id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
              status, created_at, updated_at
          ) VALUES (
              $1, 'https://issuer.test', $2, $3, '纵向测试主体',
              'active', to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(principal_id)
    .bind(format!("subject-{suffix}"))
    .bind(format!("@principal-{suffix}:matrix.test"))
    .execute(&mut **transaction)
    .await
    .expect("主体应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent (
              id, matrix_user_id, slug, display_name, visibility, lifecycle_state,
              created_at, updated_at
          ) VALUES (
              $1, $2, $3, '纵向测试 Agent', 'public', 'active',
              to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(agent_id.as_uuid())
    .bind(format!("@agent-{suffix}:matrix.test"))
    .bind(format!("agent-{}", &suffix[..24]))
    .execute(&mut **transaction)
    .await
    .expect("Agent 应创建");
    sqlx::query(
        "INSERT INTO agent_room.agent_ownership \
         (principal_id, agent_id, role, granted_by, created_at) \
         VALUES ($1, $2, 'owner', $1, to_timestamp(1700000000))",
    )
    .bind(principal_id)
    .bind(agent_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .expect("Owner 应创建");
    let device_id = Uuid::now_v7();
    let binding_id = Uuid::now_v7();
    sqlx::query(
        r"INSERT INTO agent_room.device (
              id, principal_id, label, platform, public_signing_key,
              trust_state, verified_at, created_at
          ) VALUES (
              $1, $2, '纵向测试设备', 'windows', $3, 'verified',
              to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(device_id)
    .bind(principal_id)
    .bind(test_key(device_id))
    .execute(&mut **transaction)
    .await
    .expect("设备应创建");
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
    .expect("适配器绑定应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance (
              id, agent_id, device_id, adapter_binding_id, public_signing_key,
              matrix_device_id, status, created_at
          ) VALUES (
              $1, $2, $3, $4, $5, $6, 'connecting', to_timestamp(1700000000)
          )",
    )
    .bind(agent_instance_id.as_uuid())
    .bind(agent_id.as_uuid())
    .bind(device_id)
    .bind(binding_id)
    .bind(test_key(agent_instance_id.as_uuid()))
    .bind(format!("AR_{suffix}"))
    .execute(&mut **transaction)
    .await
    .expect("Agent 实例应创建");
    DeviceId::from_uuid(device_id)
}

fn request(fixture: &Fixture) -> JoinLobbyRequest {
    JoinLobbyRequest {
        agent_id: fixture.agent,
        agent_instance_id: fixture.instance,
        catalog_id: fixture.catalog,
        mode: RoomAllocationMode::Automatic,
        preferred_language: None,
        preferred_region: None,
        evidence: RoomAllocationEvidence::default(),
    }
}

fn test_key(id: Uuid) -> Vec<u8> {
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(id.as_bytes());
    key.extend_from_slice(id.as_bytes());
    key
}

async fn catalog_space(pool: &PgPool, catalog_id: RoomCatalogId) -> Option<String> {
    sqlx::query_scalar("SELECT matrix_space_id FROM agent_room.room_catalog_entry WHERE id = $1")
        .bind(catalog_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("应能读取 Space")
}

async fn room_count(pool: &PgPool, catalog_id: RoomCatalogId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM agent_room.room_instance WHERE catalog_entry_id = $1")
        .bind(catalog_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("应能统计房间")
}

async fn committed_reservation_count(pool: &PgPool, agent_instance_id: AgentInstanceId) -> i64 {
    reservation_count(pool, agent_instance_id, "committed").await
}

async fn released_reservation_count(pool: &PgPool, agent_instance_id: AgentInstanceId) -> i64 {
    reservation_count(pool, agent_instance_id, "released").await
}

async fn reservation_count(pool: &PgPool, agent_instance_id: AgentInstanceId, state: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.room_capacity_reservation \
         WHERE agent_instance_id = $1 AND state = $2",
    )
    .bind(agent_instance_id.as_uuid())
    .bind(state)
    .fetch_one(pool)
    .await
    .expect("应能统计预约")
}

async fn allocated_slots(pool: &PgPool, catalog_id: RoomCatalogId) -> i32 {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(allocated_slots), 0)::integer \
         FROM agent_room.room_instance WHERE catalog_entry_id = $1",
    )
    .bind(catalog_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能统计槽位")
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
