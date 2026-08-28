use std::sync::{Arc, Mutex};

use agent_room_application::{
    agents::{
        AgentManagementDependencies, AgentManagementFailureKind, AgentManagementService,
        AgentManagementUseCases, CreateAgent, EnsureDefaultAgent, EnsureDefaultAgentForDevice,
        ListAgents, RegisterAgentInstance,
    },
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentCreationClaim, AgentCreationReservation, AgentCreationWorkflow,
        AgentInstanceRegistration, AgentInstanceRegistrationTransaction, AgentMembershipChange,
        AgentMembershipRepository, AgentMembershipTransaction, AgentRegistration, AgentRepository,
        Clock, IdentifierFactory, MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner,
        MatrixAgentUserRegistration, MatrixFailure, MatrixFailureKind, MatrixOperation,
        MatrixResult, MatrixSession, MatrixSessionMetadata, MatrixUserId, OutboxMessage,
        PortFuture, PrincipalAccount, RegisteredAgent, SecretDigest, SecretFactory,
        SecretGenerationFailure, SecretValue, StoredAgentInstanceRegistration,
    },
};
use agent_room_domain::{
    agents::{
        AdapterSubjectHash, Agent, AgentInstancePublicSigningKey, AgentMemberships, AgentRole,
        AgentVisibility,
    },
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

struct FakeCreationWorkflow {
    agent_id: AgentId,
    claims: Mutex<Vec<AgentCreationClaim>>,
    completions: Mutex<Vec<AgentRegistration>>,
}

impl AgentCreationWorkflow for FakeCreationWorkflow {
    fn reserve<'a>(
        &'a self,
        claim: &'a AgentCreationClaim,
    ) -> PortFuture<'a, RepositoryResult<AgentCreationReservation>> {
        Box::pin(async move {
            self.claims
                .lock()
                .expect("测试锁不得中毒")
                .push(claim.clone());
            Ok(AgentCreationReservation::Reserved {
                agent_id: self.agent_id,
            })
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

#[tokio::test]
async fn 创建_agent_先预留稳定标识再对账_matrix_身份() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let creation = Arc::new(FakeCreationWorkflow {
        agent_id,
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
        agent_id: default_agent_id,
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
        agent_id: default_agent_id,
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
        instances,
        matrix_identities: matrix,
        secrets: Arc::new(TestSecrets),
        identifiers: Arc::new(TestIdentifiers),
        clock: Arc::new(StaticClock),
    })
}

fn unused_creation(agent_id: AgentId) -> Arc<FakeCreationWorkflow> {
    Arc::new(FakeCreationWorkflow {
        agent_id,
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
