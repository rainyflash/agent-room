use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    agent_instance_management::{
        AgentInstanceCleanupFailureKind, AgentInstanceManagementDependencies,
        AgentInstanceManagementFailureKind, AgentInstanceManagementService,
        AgentInstanceManagementUseCases, AgentInstanceMatrixCleanup, ListAgentInstances,
        RevokeAgentInstance,
    },
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentInstanceManagementRecord, AgentInstanceManagementRepository,
        AgentInstanceMatrixCleanupStore, AgentInstanceRevocationOutcome,
        AgentInstanceRevocationTransaction, Clock, IdentifierFactory,
        MatrixAgentDeviceSessionRevoker, MatrixAgentDeviceSessionTarget, MatrixFailure,
        MatrixFailureKind, MatrixOperation, MatrixResult, OutboxMessage, PortFuture,
    },
};
use agent_room_domain::{
    agents::{
        AgentInstance, AgentInstancePublicSigningKey, AgentInstanceStatus, AgentMatrixDeviceId,
    },
    devices::{DevicePlatform, DeviceTrustState},
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId, WebSessionId,
    },
    time::UtcMillis,
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct StaticRuntime;

impl Clock for StaticRuntime {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

impl IdentifierFactory for StaticRuntime {
    fn principal_id(&self) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(Uuid::now_v7())
    }

    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(Uuid::now_v7())
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(Uuid::now_v7())
    }

    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(Uuid::now_v7())
    }

    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(Uuid::now_v7())
    }

    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(Uuid::now_v7())
    }

    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(Uuid::now_v7())
    }

    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(Uuid::now_v7())
    }

    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(Uuid::now_v7())
    }

    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_catalog_id(&self) -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }

    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }

    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(Uuid::now_v7())
    }

    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(Uuid::now_v7())
    }

    fn outbox_event_id(&self) -> OutboxEventId {
        OutboxEventId::from_uuid(Uuid::now_v7())
    }
}

struct FakeInstances {
    records: Vec<AgentInstanceManagementRecord>,
    list_failure: Option<RepositoryErrorKind>,
    list_calls: AtomicUsize,
}

impl AgentInstanceManagementRepository for FakeInstances {
    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentInstanceManagementRecord>>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let records = self.records.clone();
        let failure = self.list_failure;
        Box::pin(async move {
            failure.map_or_else(
                || Ok(records),
                |kind| Err(RepositoryError::new("agent_instance.list.fake", kind)),
            )
        })
    }
}

struct FakeRevocations {
    outcome: AgentInstanceRevocationOutcome,
    calls: Arc<AtomicUsize>,
    events: Mutex<Vec<OutboxMessage>>,
}

impl AgentInstanceRevocationTransaction for FakeRevocations {
    fn revoke<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _instance_id: AgentInstanceId,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentInstanceRevocationOutcome>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.events
            .lock()
            .expect("测试锁不得中毒")
            .push(event.clone());
        let outcome = self.outcome.clone();
        Box::pin(async move { Ok(outcome) })
    }
}

struct FakeMatrix {
    local_revoke_calls: Arc<AtomicUsize>,
    calls: AtomicUsize,
    targets: Mutex<Vec<MatrixAgentDeviceSessionTarget>>,
    failure: Option<MatrixFailureKind>,
}

impl MatrixAgentDeviceSessionRevoker for FakeMatrix {
    fn revoke_device_session<'a>(
        &'a self,
        target: &'a MatrixAgentDeviceSessionTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        assert_eq!(
            self.local_revoke_calls.load(Ordering::SeqCst),
            1,
            "必须先关闭本地授权边界，再调用远端"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.targets
            .lock()
            .expect("测试锁不得中毒")
            .push(target.clone());
        let failure = self.failure;
        Box::pin(async move {
            failure.map_or(Ok(()), |kind| {
                Err(MatrixFailure::new(
                    MatrixOperation::RevokeAgentDeviceSession,
                    kind,
                ))
            })
        })
    }
}

struct FakeCleanupStore {
    calls: AtomicUsize,
    fail: bool,
}

impl AgentInstanceMatrixCleanupStore for FakeCleanupStore {
    fn mark_matrix_device_revoked(
        &self,
        _instance_id: AgentInstanceId,
        _revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(RepositoryError::new(
                    "agent_instance.matrix_cleanup.fake",
                    RepositoryErrorKind::Unavailable,
                ))
            } else {
                Ok(())
            }
        })
    }
}

struct Fixture {
    service: AgentInstanceManagementService,
    instances: Arc<FakeInstances>,
    revocations: Arc<FakeRevocations>,
    matrix: Arc<FakeMatrix>,
    cleanup: Arc<FakeCleanupStore>,
}

impl Fixture {
    fn new(
        outcome: AgentInstanceRevocationOutcome,
        matrix_failure: Option<MatrixFailureKind>,
        cleanup_fails: bool,
    ) -> Self {
        let record = managed_instance(false);
        let local_revoke_calls = Arc::new(AtomicUsize::new(0));
        let instances = Arc::new(FakeInstances {
            records: vec![record],
            list_failure: None,
            list_calls: AtomicUsize::new(0),
        });
        let revocations = Arc::new(FakeRevocations {
            outcome,
            calls: local_revoke_calls.clone(),
            events: Mutex::new(Vec::new()),
        });
        let matrix = Arc::new(FakeMatrix {
            local_revoke_calls,
            calls: AtomicUsize::new(0),
            targets: Mutex::new(Vec::new()),
            failure: matrix_failure,
        });
        let cleanup = Arc::new(FakeCleanupStore {
            calls: AtomicUsize::new(0),
            fail: cleanup_fails,
        });
        let runtime = Arc::new(StaticRuntime);
        let service = AgentInstanceManagementService::new(AgentInstanceManagementDependencies {
            instances: instances.clone(),
            revocations: revocations.clone(),
            matrix_cleanup: cleanup.clone(),
            matrix: matrix.clone(),
            identifiers: runtime.clone(),
            clock: runtime,
        });
        Self {
            service,
            instances,
            revocations,
            matrix,
            cleanup,
        }
    }
}

#[tokio::test]
async fn 列表保留每个实例和所属设备的独立状态() {
    let fixture = Fixture::new(AgentInstanceRevocationOutcome::NotFound, None, false);

    let records = fixture
        .service
        .list_instances(ListAgentInstances { actor: actor(true) })
        .await
        .expect("活跃会话可以读取实例");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].device_label, "Windows 工作站");
    assert_eq!(records[0].instance.status(), AgentInstanceStatus::Revoked);
    assert_eq!(fixture.instances.list_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 撤销严格先关闭本地授权再删除_matrix_令牌并持久化结果() {
    let record = managed_instance(false);
    let fixture = Fixture::new(
        AgentInstanceRevocationOutcome::Revoked(record.clone()),
        None,
        false,
    );

    let result = fixture
        .service
        .revoke_instance(RevokeAgentInstance {
            actor: actor(true),
            instance_id: record.instance.id(),
        })
        .await
        .expect("撤销应成功");

    assert_eq!(result.matrix_cleanup, AgentInstanceMatrixCleanup::Complete);
    assert_eq!(fixture.revocations.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.matrix.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cleanup.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.revocations.events.lock().expect("测试锁不得中毒")[0].event_type(),
        "agent.instance.revoked.v1"
    );
    let target = fixture.matrix.targets.lock().expect("测试锁不得中毒")[0].clone();
    assert_eq!(target.user_id().as_str(), record.agent_matrix_user_id);
    assert_eq!(
        target.device_id().as_str(),
        record.instance.matrix_device_id().as_str()
    );
}

#[tokio::test]
async fn matrix_暂时不可用不会把已完成的本地撤销伪装成失败() {
    let record = managed_instance(false);
    let fixture = Fixture::new(
        AgentInstanceRevocationOutcome::Revoked(record.clone()),
        Some(MatrixFailureKind::DependencyUnavailable),
        false,
    );

    let result = fixture
        .service
        .revoke_instance(RevokeAgentInstance {
            actor: actor(true),
            instance_id: record.instance.id(),
        })
        .await
        .expect("本地撤销已经失败关闭");

    assert_eq!(
        result.matrix_cleanup,
        AgentInstanceMatrixCleanup::Pending {
            reason: AgentInstanceCleanupFailureKind::DependencyUnavailable,
        }
    );
    assert_eq!(fixture.cleanup.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 已记录远端清理的重复撤销不会再次调用_matrix() {
    let record = managed_instance(true);
    let fixture = Fixture::new(
        AgentInstanceRevocationOutcome::AlreadyRevoked(record.clone()),
        None,
        false,
    );

    let result = fixture
        .service
        .revoke_instance(RevokeAgentInstance {
            actor: actor(true),
            instance_id: record.instance.id(),
        })
        .await
        .expect("重复撤销必须幂等");

    assert_eq!(result.matrix_cleanup, AgentInstanceMatrixCleanup::Complete);
    assert_eq!(fixture.matrix.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.cleanup.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 过期网页会话在访问仓储前失败关闭() {
    let fixture = Fixture::new(AgentInstanceRevocationOutcome::NotFound, None, false);

    let failure = fixture
        .service
        .list_instances(ListAgentInstances {
            actor: actor(false),
        })
        .await
        .expect_err("过期会话必须被拒绝");

    assert_eq!(
        failure.kind(),
        AgentInstanceManagementFailureKind::Forbidden
    );
    assert_eq!(fixture.instances.list_calls.load(Ordering::SeqCst), 0);
}

fn managed_instance(matrix_device_revoked: bool) -> AgentInstanceManagementRecord {
    let instance = AgentInstance::restore(
        agent_instance_id(),
        agent_id(),
        device_id(),
        adapter_binding_id(),
        AgentInstancePublicSigningKey::new(vec![7; 32]).expect("公钥有效"),
        AgentMatrixDeviceId::new("AR_TEST_INSTANCE".to_owned()).expect("设备标识有效"),
        AgentInstanceStatus::Revoked,
        None,
    )
    .expect("实例快照有效");
    AgentInstanceManagementRecord {
        instance,
        agent_matrix_user_id: format!("@_agent_{}:matrix.agent-room.localhost", agent_id()),
        agent_display_name: "审阅 Agent".to_owned(),
        agent_avatar_content_id: None,
        adapter_type: "codex".to_owned(),
        capability_version: "1".to_owned(),
        device_label: "Windows 工作站".to_owned(),
        device_platform: DevicePlatform::Windows,
        device_trust_state: DeviceTrustState::Verified,
        created_at: time(NOW - 10_000),
        last_seen_at: Some(time(NOW - 1_000)),
        revoked_at: Some(time(NOW)),
        matrix_device_revoked_at: matrix_device_revoked.then(|| time(NOW)),
    }
}

fn actor(active: bool) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal_id(),
        matrix_user_id: "@operator:matrix.agent-room.localhost".to_owned(),
        display_name: "操作人".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(if active { NOW + 60_000 } else { NOW }),
        recently_authenticated: true,
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid("01945c1e-7b5a-7000-8000-000000000001"))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid("01945c1e-7b5a-7000-8000-000000000002"))
}

fn agent_instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid("01945c1e-7b5a-7000-8000-000000000003"))
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(uuid("01945c1e-7b5a-7000-8000-000000000004"))
}

fn adapter_binding_id() -> AdapterBindingId {
    AdapterBindingId::from_uuid(uuid("01945c1e-7b5a-7000-8000-000000000005"))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
