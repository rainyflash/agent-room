use std::sync::{Arc, Mutex};

use agent_room_application::{
    agents::{
        AgentManagementDependencies, AgentManagementFailureKind, AgentManagementService,
        AgentManagementUseCases, CreateAgent, CreateHostAgentForDevice, EnsureDefaultAgent,
        EnsureDefaultAgentForDevice, ListAgents, RegisterAgentInstance,
        RotateAgentInstanceMatrixSession,
    },
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentCreationClaim, AgentCreationReservation, AgentCreationWorkflow,
        AgentInstanceManagementRecord, AgentInstanceManagementRepository,
        AgentInstanceRegistration, AgentInstanceRegistrationTransaction, AgentMembershipChange,
        AgentMembershipRepository, AgentMembershipTransaction, AgentRegistration, AgentRepository,
        Clock, IdentifierFactory, MatrixAgentDeviceSessionRequest, MatrixAgentDeviceSessionRotator,
        MatrixAgentIdentityProvisioner, MatrixAgentUserRegistration, MatrixFailure,
        MatrixFailureKind, MatrixOperation, MatrixResult, MatrixSession, MatrixSessionMetadata,
        MatrixUserId, OutboxMessage, PortFuture, PrincipalAccount, RegisteredAgent, SecretDigest,
        SecretFactory, SecretGenerationFailure, SecretValue, StoredAgentInstanceRegistration,
    },
};
use agent_room_domain::{
    agents::{
        AdapterSubjectHash, Agent, AgentInstance, AgentInstancePublicSigningKey,
        AgentInstanceStatus, AgentMatrixDeviceId, AgentMemberships, AgentRole, AgentVisibility,
    },
    devices::{DevicePlatform, DeviceTrustState},
    identity::Principal,
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentCreationRequestId, AgentId, AgentInstanceId,
        AgentInstanceRegistrationRequestId, AutomationGrantId, ContentId, DeviceAccessTokenId,
        DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId, HandoffId, LoginAttemptId,
        OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomReservationId, WebSessionId,
    },
    time::UtcMillis,
};
use serde_json::Map;
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct StaticClock;

impl Clock for StaticClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct TestIdentifiers;

impl IdentifierFactory for TestIdentifiers {
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

struct TestSecrets;

impl SecretFactory for TestSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        SecretValue::new("test-secret-with-at-least-256-bits-of-fixture-entropy")
            .map_err(|_| SecretGenerationFailure::EntropyUnavailable)
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut digest = [0_u8; 32];
        for (index, byte) in value.bytes().enumerate() {
            let slot = index % digest.len();
            digest[slot] = digest[slot].wrapping_mul(31).wrapping_add(byte);
        }
        SecretDigest::from_array(digest)
    }
}

#[derive(Default)]
struct FakeCreationWorkflow {
    agent_id: Option<AgentId>,
    claims: Mutex<Vec<AgentCreationClaim>>,
    completions: Mutex<Vec<AgentRegistration>>,
}

impl AgentCreationWorkflow for FakeCreationWorkflow {
    fn reserve<'a>(
        &'a self,
        claim: &'a AgentCreationClaim,
    ) -> PortFuture<'a, RepositoryResult<AgentCreationReservation>> {
        Box::pin(async move {
            let mut claims = self.claims.lock().expect("测试锁不得中毒");
            let first = claims
                .iter()
                .find(|previous| previous.request_id == claim.request_id)
                .cloned();
            claims.push(claim.clone());
            let reserved_id = if let Some(first) = first {
                if first.owner_id != claim.owner_id {
                    return Err(RepositoryError::new(
                        "agent_creation.reserve",
                        RepositoryErrorKind::Forbidden,
                    ));
                }
                if first.request_fingerprint != claim.request_fingerprint {
                    return Err(RepositoryError::new(
                        "agent_creation.reserve",
                        RepositoryErrorKind::Conflict,
                    ));
                }
                first.proposed_agent_id
            } else {
                claim.proposed_agent_id
            };
            let agent_id = self.agent_id.unwrap_or(reserved_id);
            let completed = self
                .completions
                .lock()
                .expect("测试锁不得中毒")
                .iter()
                .find(|registration| registration.agent.id() == agent_id)
                .map(RegisteredAgent::from);
            Ok(completed.map_or(
                AgentCreationReservation::Reserved { agent_id },
                AgentCreationReservation::Completed,
            ))
        })
    }

    fn complete_with_event<'a>(
        &'a self,
        _request_id: AgentCreationRequestId,
        _request_fingerprint: &'a SecretDigest,
        registration: &'a AgentRegistration,
        _event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move {
            self.completions
                .lock()
                .expect("测试锁不得中毒")
                .push(registration.clone());
            Ok(registration.agent.clone())
        })
    }
}

struct FakeAgentRepository {
    registration: Option<RegisteredAgent>,
}

impl AgentRepository for FakeAgentRepository {
    fn find(&self, id: AgentId) -> PortFuture<'_, RepositoryResult<Option<Agent>>> {
        let value = self
            .registration
            .as_ref()
            .filter(|registration| registration.agent.id() == id)
            .map(|registration| registration.agent.clone());
        Box::pin(async move { Ok(value) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RegisteredAgent>>> {
        let value = self.registration.clone().into_iter().collect();
        Box::pin(async move { Ok(value) })
    }

    fn find_registration(
        &self,
        id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<RegisteredAgent>>> {
        let value = self
            .registration
            .as_ref()
            .filter(|registration| registration.agent.id() == id)
            .cloned();
        Box::pin(async move { Ok(value) })
    }

    fn create<'a>(
        &'a self,
        _registration: &'a AgentRegistration,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move { Err(unavailable("agent.create.unused")) })
    }

    fn save<'a>(&'a self, _agent: &'a Agent) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move { Err(unavailable("agent.save.unused")) })
    }
}

struct FakeMemberships {
    memberships: Option<AgentMemberships>,
}

impl AgentMembershipRepository for FakeMemberships {
    fn find_memberships(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentMemberships>>> {
        let value = self.memberships.clone();
        Box::pin(async move { Ok(value) })
    }
}

struct FakeMembershipChanges;

impl AgentMembershipTransaction for FakeMembershipChanges {
    fn apply_change<'a>(
        &'a self,
        _change: &'a AgentMembershipChange,
        _event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<AgentMemberships>> {
        Box::pin(async move { Err(unavailable("agent_membership.change.unused")) })
    }
}

#[derive(Default)]
struct FakeInstances {
    registrations: Mutex<Vec<AgentInstanceRegistration>>,
    active_instance: Mutex<Option<AgentInstanceManagementRecord>>,
}

impl AgentInstanceRegistrationTransaction for FakeInstances {
    fn register_with_event<'a>(
        &'a self,
        registration: &'a AgentInstanceRegistration,
        _event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<StoredAgentInstanceRegistration>> {
        Box::pin(async move {
            self.registrations
                .lock()
                .expect("测试锁不得中毒")
                .push(registration.clone());
            Ok(StoredAgentInstanceRegistration {
                binding: registration.binding.clone(),
                instance: registration.instance.clone(),
            })
        })
    }
}

impl AgentInstanceManagementRepository for FakeInstances {
    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentInstanceManagementRecord>>> {
        Box::pin(async { unreachable!("Agent 管理测试不会列出实例") })
    }

    fn find_active_for_device(
        &self,
        _principal_id: PrincipalId,
        _device_id: DeviceId,
        _instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceManagementRecord>>> {
        let record = self.active_instance.lock().expect("活跃实例锁可用").clone();
        Box::pin(async move { Ok(record) })
    }
}

struct FakeMatrixIdentities {
    server_name: String,
    issued_sessions: Mutex<usize>,
    corrupt_session_identity: bool,
}

impl MatrixAgentIdentityProvisioner for FakeMatrixIdentities {
    fn ensure_user<'a>(
        &'a self,
        registration: &'a MatrixAgentUserRegistration,
    ) -> PortFuture<'a, MatrixResult<MatrixUserId>> {
        Box::pin(async move {
            MatrixUserId::new(format!(
                "@{}:{}",
                registration.localpart().as_str(),
                self.server_name
            ))
            .map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::ProvisionAgentUser,
                    MatrixFailureKind::InvalidResponse,
                )
            })
        })
    }

    fn issue_device_session<'a>(
        &'a self,
        request: &'a MatrixAgentDeviceSessionRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>> {
        Box::pin(async move {
            *self.issued_sessions.lock().expect("测试锁不得中毒") += 1;
            let user_id = if self.corrupt_session_identity {
                MatrixUserId::new("@_agent_00000000000000000000000000000000:matrix.test")
                    .expect("伪造用户标识有效")
            } else {
                request.user_id().clone()
            };
            Ok(MatrixSession::new(
                MatrixSessionMetadata::new(user_id, request.device_id().clone()),
                SecretValue::new("agent-device-session-token").expect("测试 Token 有效"),
                None,
            ))
        })
    }
}

impl MatrixAgentDeviceSessionRotator for FakeMatrixIdentities {
    fn rotate_device_session<'a>(
        &'a self,
        request: &'a MatrixAgentDeviceSessionRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>> {
        Box::pin(async move {
            *self.issued_sessions.lock().expect("测试锁不得中毒") += 1;
            let user_id = if self.corrupt_session_identity {
                MatrixUserId::new("@_agent_00000000000000000000000000000000:matrix.test")
                    .expect("伪造用户标识有效")
            } else {
                request.user_id().clone()
            };
            Ok(MatrixSession::new(
                MatrixSessionMetadata::new(user_id, request.device_id().clone()),
                SecretValue::new("rotated-agent-device-session-token").expect("测试 Token 有效"),
                None,
            ))
        })
    }
}

#[tokio::test]
async fn 创建_agent_先预留稳定标识再对账_matrix_身份() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let creation = Arc::new(FakeCreationWorkflow {
        agent_id: Some(agent_id),
        claims: Mutex::new(Vec::new()),
        completions: Mutex::new(Vec::new()),
    });
    let matrix = Arc::new(FakeMatrixIdentities {
        server_name: "matrix.test".to_owned(),
        issued_sessions: Mutex::new(0),
        corrupt_session_identity: false,
    });
    let service = service(
        creation.clone(),
        None,
        None,
        Arc::new(FakeInstances::default()),
        matrix,
    );
    let actor = authenticated_principal(PrincipalId::from_uuid(Uuid::now_v7()));
    let created = service
        .create_agent(CreateAgent {
            request_id: AgentCreationRequestId::from_uuid(Uuid::now_v7()),
            actor: actor.clone(),
            slug: "build-agent".to_owned(),
            display_name: "Build Agent".to_owned(),
            description: "CI helper".to_owned(),
            avatar_content_id: None,
            visibility: AgentVisibility::Private,
        })
        .await
        .expect("Agent 创建应成功");

    assert_eq!(created.agent.id(), agent_id);
    assert_eq!(
        created.matrix_user_id,
        format!("@_agent_{}:matrix.test", agent_id.as_uuid().simple())
    );
    let claims = creation.claims.lock().expect("测试锁不得中毒");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].owner_id, actor.principal_id);
    assert_eq!(
        creation.completions.lock().expect("测试锁不得中毒").len(),
        1
    );
}

#[tokio::test]
async fn 首次引导按_principal_幂等创建默认_agent() {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let default_agent_id = AgentId::from_uuid(principal_id.as_uuid());
    let creation = Arc::new(FakeCreationWorkflow {
        agent_id: Some(default_agent_id),
        claims: Mutex::new(Vec::new()),
        completions: Mutex::new(Vec::new()),
    });
    let service = service(
        creation.clone(),
        None,
        None,
        Arc::new(FakeInstances::default()),
        Arc::new(FakeMatrixIdentities {
            server_name: "matrix.test".to_owned(),
            issued_sessions: Mutex::new(0),
            corrupt_session_identity: false,
        }),
    );

    let ensured = service
        .ensure_default_agent(EnsureDefaultAgent {
            actor: authenticated_principal(principal_id),
        })
        .await
        .expect("默认 Agent 应创建成功");

    assert_eq!(ensured.agent.id(), default_agent_id);
    let claims = creation.claims.lock().expect("测试锁不得中毒");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].request_id.as_uuid(), principal_id.as_uuid());
    assert_eq!(claims[0].proposed_agent_id, default_agent_id);
}

#[tokio::test]
async fn 设备身份首次引导复用同一_principal_确定性_agent() {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let default_agent_id = AgentId::from_uuid(principal_id.as_uuid());
    let creation = Arc::new(FakeCreationWorkflow {
        agent_id: Some(default_agent_id),
        claims: Mutex::new(Vec::new()),
        completions: Mutex::new(Vec::new()),
    });
    let service = service(
        creation.clone(),
        None,
        None,
        Arc::new(FakeInstances::default()),
        Arc::new(FakeMatrixIdentities {
            server_name: "matrix.test".to_owned(),
            issued_sessions: Mutex::new(0),
            corrupt_session_identity: false,
        }),
    );

    let ensured = service
        .ensure_default_agent_for_device(EnsureDefaultAgentForDevice {
            actor: authenticated_device(principal_id),
        })
        .await
        .expect("设备身份应可确保默认 Agent");

    assert_eq!(ensured.agent.id(), default_agent_id);
    let claims = creation.claims.lock().expect("测试锁不得中毒");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].owner_id, principal_id);
    assert_eq!(claims[0].request_id.as_uuid(), principal_id.as_uuid());
    assert_eq!(claims[0].proposed_agent_id, default_agent_id);
}

#[tokio::test]
async fn 已有_agent_时首次引导直接复用且不创建重复记录() {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let existing = registered_agent(AgentId::from_uuid(Uuid::now_v7()));
    let creation = unused_creation(AgentId::from_uuid(Uuid::now_v7()));
    let service = service(
        creation.clone(),
        Some(existing.clone()),
        None,
        Arc::new(FakeInstances::default()),
        Arc::new(FakeMatrixIdentities {
            server_name: "matrix.test".to_owned(),
            issued_sessions: Mutex::new(0),
            corrupt_session_identity: false,
        }),
    );

    let listed = service
        .list_agents(ListAgents {
            actor: authenticated_principal(principal_id),
        })
        .await
        .expect("Agent 列表应可读取");
    let ensured = service
        .ensure_default_agent(EnsureDefaultAgent {
            actor: authenticated_principal(principal_id),
        })
        .await
        .expect("已有 Agent 应被复用");

    assert_eq!(listed, vec![existing.clone()]);
    assert_eq!(ensured, existing);
    assert!(creation.claims.lock().expect("测试锁不得中毒").is_empty());
}

#[tokio::test]
async fn 宿主会话创建独立_agent_且重试复用同一身份() {
    let creation = Arc::new(FakeCreationWorkflow::default());
    let service = host_agent_service(creation.clone());
    let request = host_agent_request("调试 Agent A");
    let first = service
        .create_host_agent_for_device(request.clone())
        .await
        .expect("已认证设备可创建宿主 Agent");
    let replay = service
        .create_host_agent_for_device(request.clone())
        .await
        .expect("相同会话和参数可安全重试");
    let second = service
        .create_host_agent_for_device(CreateHostAgentForDevice {
            request_id: AgentCreationRequestId::from_uuid(Uuid::now_v7()),
            ..request.clone()
        })
        .await
        .expect("同一设备的另一个宿主会话可创建独立 Agent");

    assert_eq!(replay, first);
    assert_ne!(first.agent.id(), second.agent.id());
    assert_ne!(first.matrix_user_id, second.matrix_user_id);
    assert_ne!(first.slug, second.slug);
    assert_ne!(first.agent.id().as_uuid(), request.request_id.as_uuid());
    assert_ne!(
        first.agent.id().as_uuid(),
        request.actor.account.principal.id().as_uuid()
    );
    assert_eq!(first.display_name, request.display_name);
    assert_eq!(first.visibility, AgentVisibility::Private);
    assert!(first.description.is_empty());
    assert!(first.avatar_content_id.is_none());
    let claims = creation.claims.lock().expect("测试锁不得中毒");
    assert_eq!(claims.len(), 3);
    assert_eq!(claims[0].request_id, request.request_id);
    assert_eq!(claims[0].owner_id, request.actor.account.principal.id());
    assert_eq!(claims[0].proposed_agent_id, first.agent.id());
    assert_eq!(claims[0].request_fingerprint, claims[1].request_fingerprint);
    assert_ne!(claims[0].proposed_agent_id, claims[1].proposed_agent_id);
    assert_eq!(
        creation.completions.lock().expect("测试锁不得中毒").len(),
        2
    );
}

#[tokio::test]
async fn 宿主会话拒绝改名冲突和跨所有者重用() {
    let creation = Arc::new(FakeCreationWorkflow::default());
    let service = host_agent_service(creation.clone());
    let request = host_agent_request("调试 Agent A");
    let first = service
        .create_host_agent_for_device(request.clone())
        .await
        .expect("首次创建成功");
    let conflict = service
        .create_host_agent_for_device(CreateHostAgentForDevice {
            display_name: "调试 Agent B".to_owned(),
            ..request.clone()
        })
        .await
        .expect_err("同一会话不得更改创建参数");
    assert_eq!(conflict.kind(), AgentManagementFailureKind::Conflict);
    let forbidden = service
        .create_host_agent_for_device(CreateHostAgentForDevice {
            actor: authenticated_device(PrincipalId::from_uuid(Uuid::now_v7())),
            ..request.clone()
        })
        .await
        .expect_err("其他所有者不得认领同一会话");
    assert_eq!(forbidden.kind(), AgentManagementFailureKind::Forbidden);
    let replay = service
        .create_host_agent_for_device(request)
        .await
        .expect("拒绝冲突后原所有者仍可重试");
    assert_eq!(replay, first);
    assert_eq!(
        creation.completions.lock().expect("测试锁不得中毒").len(),
        1
    );
}

#[tokio::test]
async fn 宿主_agent_沿用显示名称规则且无效输入不预留身份() {
    let creation = Arc::new(FakeCreationWorkflow::default());
    let service = host_agent_service(creation.clone());
    for name in [
        String::new(),
        "x".repeat(129),
        "Agent\nA".to_owned(),
        "测".repeat(43),
    ] {
        let failure = service
            .create_host_agent_for_device(host_agent_request(&name))
            .await
            .expect_err("空名称、超长名称和控制字符必须失败");
        assert_eq!(failure.kind(), AgentManagementFailureKind::InvalidRequest);
    }
    assert!(creation.claims.lock().expect("测试锁不得中毒").is_empty());
    let maximum = "x".repeat(128);
    let created = service
        .create_host_agent_for_device(host_agent_request(&maximum))
        .await
        .expect("规则允许的最大长度应被接受");
    assert_eq!(created.display_name, maximum);
}

#[tokio::test]
async fn 过期设备认证或暂停主体不能创建或重放宿主_agent() {
    let creation = Arc::new(FakeCreationWorkflow::default());
    let service = host_agent_service(creation.clone());
    let request = host_agent_request("调试 Agent A");
    service
        .create_host_agent_for_device(request.clone())
        .await
        .expect("原始有效认证可创建 Agent");
    let mut expired = request.clone();
    expired.actor.access_token_expires_at = time(NOW);
    let mut suspended = request;
    suspended
        .actor
        .account
        .principal
        .suspend()
        .expect("主体可暂停");
    for denied in [expired, suspended] {
        let failure = service
            .create_host_agent_for_device(denied)
            .await
            .expect_err("认证失效后不得重放已完成请求");
        assert_eq!(failure.kind(), AgentManagementFailureKind::Forbidden);
    }
    assert_eq!(creation.claims.lock().expect("测试锁不得中毒").len(), 1);
}

fn host_agent_service(creation: Arc<FakeCreationWorkflow>) -> AgentManagementService {
    service(
        creation,
        Some(registered_agent(AgentId::from_uuid(Uuid::now_v7()))),
        None,
        Arc::new(FakeInstances::default()),
        Arc::new(FakeMatrixIdentities {
            server_name: "matrix.test".to_owned(),
            issued_sessions: Mutex::new(0),
            corrupt_session_identity: false,
        }),
    )
}

fn host_agent_request(display_name: &str) -> CreateHostAgentForDevice {
    CreateHostAgentForDevice {
        request_id: AgentCreationRequestId::from_uuid(Uuid::now_v7()),
        actor: authenticated_device(PrincipalId::from_uuid(Uuid::now_v7())),
        display_name: display_name.to_owned(),
    }
}

#[tokio::test]
async fn viewer_在写事务和_matrix_签发前即被拒绝() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let viewer_id = PrincipalId::from_uuid(Uuid::now_v7());
    let mut memberships = AgentMemberships::with_initial_owner(agent_id, owner_id, time(NOW));
    memberships
        .grant_role(owner_id, viewer_id, AgentRole::Viewer, time(NOW))
        .expect("测试 Viewer 授权有效");
    let instances = Arc::new(FakeInstances::default());
    let matrix = Arc::new(FakeMatrixIdentities {
        server_name: "matrix.test".to_owned(),
        issued_sessions: Mutex::new(0),
        corrupt_session_identity: false,
    });
    let service = service(
        unused_creation(agent_id),
        Some(registered_agent(agent_id)),
        Some(memberships),
        instances.clone(),
        matrix.clone(),
    );

    let error = service
        .register_instance(register_request(viewer_id, agent_id))
        .await
        .expect_err("Viewer 不得注册实例");
    assert_eq!(error.kind(), AgentManagementFailureKind::Forbidden);
    assert!(
        instances
            .registrations
            .lock()
            .expect("测试锁不得中毒")
            .is_empty()
    );
    assert_eq!(*matrix.issued_sessions.lock().expect("测试锁不得中毒"), 0);
}

#[tokio::test]
async fn 会话恢复按现有实例直接轮换而不重放注册请求() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let actor = authenticated_device(owner_id);
    let instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
    let instances = Arc::new(FakeInstances::default());
    *instances.active_instance.lock().expect("活跃实例锁可用") =
        Some(managed_instance(agent_id, actor.device_id, instance_id));
    let matrix = Arc::new(FakeMatrixIdentities {
        server_name: "matrix.test".to_owned(),
        issued_sessions: Mutex::new(0),
        corrupt_session_identity: false,
    });
    let service = service(
        unused_creation(agent_id),
        Some(registered_agent(agent_id)),
        None,
        instances,
        matrix.clone(),
    );

    let rotated = service
        .rotate_instance_matrix_session(RotateAgentInstanceMatrixSession { actor, instance_id })
        .await
        .expect("同一设备持有的活跃实例可轮换 Matrix 会话");

    assert_eq!(rotated.instance.instance.id(), instance_id);
    assert_eq!(
        rotated.matrix_session.access_token().expose(),
        "rotated-agent-device-session-token"
    );
    assert_eq!(*matrix.issued_sessions.lock().expect("测试锁不得中毒"), 1);
}

#[tokio::test]
async fn matrix_返回错主体时拒绝把凭据交给_bridge() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let memberships = AgentMemberships::with_initial_owner(agent_id, owner_id, time(NOW));
    let matrix = Arc::new(FakeMatrixIdentities {
        server_name: "matrix.test".to_owned(),
        issued_sessions: Mutex::new(0),
        corrupt_session_identity: true,
    });
    let service = service(
        unused_creation(agent_id),
        Some(registered_agent(agent_id)),
        Some(memberships),
        Arc::new(FakeInstances::default()),
        matrix,
    );

    let error = service
        .register_instance(register_request(owner_id, agent_id))
        .await
        .expect_err("错误 Matrix 主体不得通过边界");
    assert_eq!(error.kind(), AgentManagementFailureKind::Internal);
}

fn service(
    creations: Arc<FakeCreationWorkflow>,
    registration: Option<RegisteredAgent>,
    memberships: Option<AgentMemberships>,
    instances: Arc<FakeInstances>,
    matrix: Arc<FakeMatrixIdentities>,
) -> AgentManagementService {
    AgentManagementService::new(AgentManagementDependencies {
        creations,
        agents: Arc::new(FakeAgentRepository { registration }),
        memberships: Arc::new(FakeMemberships { memberships }),
        membership_changes: Arc::new(FakeMembershipChanges),
        instances: instances.clone(),
        managed_instances: instances,
        matrix_identities: matrix.clone(),
        matrix_sessions: matrix,
        secrets: Arc::new(TestSecrets),
        identifiers: Arc::new(TestIdentifiers),
        clock: Arc::new(StaticClock),
    })
}

fn unused_creation(agent_id: AgentId) -> Arc<FakeCreationWorkflow> {
    Arc::new(FakeCreationWorkflow {
        agent_id: Some(agent_id),
        claims: Mutex::new(Vec::new()),
        completions: Mutex::new(Vec::new()),
    })
}

fn registered_agent(agent_id: AgentId) -> RegisteredAgent {
    RegisteredAgent {
        agent: Agent::register(agent_id),
        matrix_user_id: format!("@_agent_{}:matrix.test", agent_id.as_uuid().simple()),
        slug: "build-agent".to_owned(),
        display_name: "Build Agent".to_owned(),
        description: String::new(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: time(NOW),
    }
}

fn register_request(principal_id: PrincipalId, agent_id: AgentId) -> RegisterAgentInstance {
    RegisterAgentInstance {
        request_id: AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        actor: authenticated_device(principal_id),
        agent_id,
        adapter_type: "codex".to_owned(),
        external_subject_hash: Some(AdapterSubjectHash::new(vec![7; 32]).expect("主体摘要有效")),
        capability_version: "1.0".to_owned(),
        configuration: Map::new(),
        public_signing_key: AgentInstancePublicSigningKey::new(vec![9; 32]).expect("实例公钥有效"),
    }
}

fn managed_instance(
    agent_id: AgentId,
    device_id: DeviceId,
    instance_id: AgentInstanceId,
) -> AgentInstanceManagementRecord {
    let binding_id = AdapterBindingId::from_uuid(Uuid::now_v7());
    let matrix_device_id =
        AgentMatrixDeviceId::new("AR_TEST".to_owned()).expect("Matrix 设备标识有效");
    let instance = AgentInstance::restore(
        instance_id,
        agent_id,
        device_id,
        binding_id,
        AgentInstancePublicSigningKey::new(vec![9; 32]).expect("实例公钥有效"),
        matrix_device_id,
        AgentInstanceStatus::Connecting,
        None,
    )
    .expect("实例记录有效");
    AgentInstanceManagementRecord {
        instance,
        agent_matrix_user_id: format!("@_agent_{}:matrix.test", agent_id.as_uuid().simple()),
        agent_display_name: "Build Agent".to_owned(),
        agent_avatar_content_id: None,
        adapter_type: "codex".to_owned(),
        capability_version: "1.0".to_owned(),
        device_label: "Windows 工作站".to_owned(),
        device_platform: DevicePlatform::Windows,
        device_trust_state: DeviceTrustState::Verified,
        created_at: time(NOW - 60_000),
        last_seen_at: None,
        revoked_at: None,
        matrix_device_revoked_at: None,
    }
}

fn authenticated_device(principal_id: PrincipalId) -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(principal_id),
            matrix_user_id: format!("@principal-{principal_id}:matrix.test"),
            display_name: "测试用户".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: DeviceId::from_uuid(Uuid::now_v7()),
        access_token_expires_at: time(NOW + 60_000),
    }
}

fn authenticated_principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: format!("@principal-{principal_id}:matrix.test"),
        display_name: "测试用户".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: true,
    }
}

fn unavailable(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Unavailable)
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
