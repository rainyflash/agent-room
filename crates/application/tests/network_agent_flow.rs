use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use agent_room_application::{
    agent_lobbies::{
        AgentLobbyEntryFailure, AgentLobbyEntryResult, AgentLobbyEntryUseCases, EnterAgentLobby,
    },
    agents::{
        AgentManagementResult, AgentManagementUseCases, ChangeAgentMembership, CreateAgent,
        CreateHostAgentForDevice, DeleteAgent, EnsureDefaultAgent, EnsureDefaultAgentForDevice,
        ListAgents, RegisterAgentInstance, RegisteredAgentInstance,
        RotateAgentInstanceMatrixSession, RotatedAgentInstanceMatrixSession,
    },
    network_agents::{
        CreateNetworkAgent, NetworkAgentDependencies, NetworkAgentFailureKind,
        NetworkAgentPendingExit, NetworkAgentPolicy, NetworkAgentService, NetworkAgentUseCases,
    },
    persistence::RepositoryResult,
    ports::{
        Clock, GeneratedSigningKey, IdentifierFactory, MatrixDeviceId, MatrixSession,
        MatrixSessionMetadata, MatrixUserId, NetworkAgentActivation, NetworkAgentBeginOutcome,
        NetworkAgentKeyFactory, NetworkAgentPause, NetworkAgentProvisioning, NetworkAgentRecord,
        NetworkAgentRoomRecord, NetworkAgentSecretKind, NetworkAgentSecretSealer,
        NetworkAgentStaleCutoff, NetworkAgentStore, PortFuture, PublicLobbyDirectoryEntry,
        PublicLobbyObservationRoom, RateWindowDecision, RateWindowPolicy, RegisteredAgent,
        RoomDirectory, RoomDirectoryQuery, SealedSecret, SecretDigest, SecretFactory,
        SecretGenerationFailure, SecretSealingFailure, SecretValue,
        StoredAgentInstanceRegistration,
    },
    rooms::{EnterLobbyOutcome, LobbyJoinKind},
};
use agent_room_domain::{
    agents::{
        AdapterBinding, Agent, AgentInstance, AgentInstanceStatus, AgentMatrixDeviceId,
        AgentVisibility,
    },
    devices::{DevicePlatform, DeviceTrustState},
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, NetworkAgentId, OutboxEventId, PrincipalId, RoomCatalogId,
        RoomInstanceId, RoomReservationId, WebSessionId,
    },
    network_agents::{NETWORK_AGENT_ISSUER, NetworkAgentStatus},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomReservation, RoomReservationFields, RoomReservationState, RoomSlug,
    },
    time::UtcMillis,
};
use uuid::Uuid;

const START: i64 = 1_758_600_000_000;
const HOUR: i64 = 60 * 60 * 1_000;
const MATRIX_TOKEN: &str = "syt_network_agent_session";

// ---------- 假实现 ----------

struct StoredAgent {
    record: NetworkAgentRecord,
    token_digest: SecretDigest,
    principal: agent_room_application::ports::PrincipalRegistration,
    device_platform: DevicePlatform,
    device_trust: DeviceTrustState,
    secrets: Vec<(NetworkAgentSecretKind, SealedSecret)>,
    /// 停用后离开了所有房间（或从没进过）。
    rooms_left: bool,
}

#[derive(Default)]
struct MemoryStore {
    agents: Mutex<Vec<StoredAgent>>,
    windows: Mutex<HashMap<String, (UtcMillis, u32)>>,
    rooms: Mutex<Vec<(NetworkAgentId, NetworkAgentRoomRecord)>>,
}

impl MemoryStore {
    fn only(&self) -> NetworkAgentRecord {
        let agents = self.agents.lock().unwrap();
        assert_eq!(agents.len(), 1, "应当只有一个网络 Agent");
        agents[0].record.clone()
    }

    fn with<T>(&self, id: NetworkAgentId, read: impl FnOnce(&StoredAgent) -> T) -> T {
        let agents = self.agents.lock().unwrap();
        read(agents.iter().find(|agent| agent.record.id == id).unwrap())
    }

    fn statuses(&self) -> Vec<(String, NetworkAgentStatus)> {
        self.agents
            .lock()
            .unwrap()
            .iter()
            .map(|agent| (agent.record.display_name.clone(), agent.record.status))
            .collect()
    }
}

impl NetworkAgentStore for MemoryStore {
    fn begin(
        &self,
        provisioning: &NetworkAgentProvisioning,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentBeginOutcome>> {
        let mut agents = self.agents.lock().unwrap();
        let taken = agents.iter().any(|agent| {
            agent.record.status != NetworkAgentStatus::Disabled
                && agent.record.display_name.to_lowercase()
                    == provisioning.display_name.to_lowercase()
        });
        let outcome = if taken {
            NetworkAgentBeginOutcome::NameTaken
        } else {
            agents.push(StoredAgent {
                record: NetworkAgentRecord {
                    id: provisioning.id,
                    principal_id: provisioning.principal.principal.id(),
                    device_id: provisioning.device.id(),
                    agent_id: None,
                    agent_instance_id: None,
                    display_name: provisioning.display_name.clone(),
                    status: NetworkAgentStatus::Provisioning,
                    created_at: provisioning.created_at,
                    last_active_at: provisioning.created_at,
                },
                token_digest: provisioning.token_digest,
                principal: provisioning.principal.clone(),
                device_platform: provisioning.device.platform(),
                device_trust: provisioning.device.trust_state(),
                secrets: provisioning.secrets.clone(),
                rooms_left: false,
            });
            NetworkAgentBeginOutcome::Created
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn activate(
        &self,
        activation: &NetworkAgentActivation,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut agents = self.agents.lock().unwrap();
        let agent = agents
            .iter_mut()
            .find(|agent| agent.record.id == activation.id)
            .unwrap();
        agent.record.agent_id = Some(activation.agent_id);
        agent.record.agent_instance_id = Some(activation.agent_instance_id);
        agent.record.status = NetworkAgentStatus::Active;
        agent.secrets.push((
            NetworkAgentSecretKind::MatrixAccessToken,
            activation.matrix_access_token.clone(),
        ));
        Box::pin(async { Ok(()) })
    }

    fn find_by_token<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<NetworkAgentRecord>>> {
        let found = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .find(|agent| &agent.token_digest == digest)
            .map(|agent| agent.record.clone());
        Box::pin(async move { Ok(found) })
    }

    fn find_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedSecret>>> {
        let found = self.with(id, |agent| {
            agent
                .secrets
                .iter()
                .find(|(stored, _)| *stored == kind)
                .map(|(_, sealed)| sealed.clone())
        });
        Box::pin(async move { Ok(found) })
    }

    fn count_live(&self) -> PortFuture<'_, RepositoryResult<u64>> {
        let count = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .filter(|agent| agent.record.status != NetworkAgentStatus::Disabled)
            .count();
        Box::pin(async move { Ok(u64::try_from(count).unwrap()) })
    }

    fn record_activity(
        &self,
        id: NetworkAgentId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut agents = self.agents.lock().unwrap();
        let agent = agents
            .iter_mut()
            .find(|agent| agent.record.id == id)
            .unwrap();
        agent.record.last_active_at = agent.record.last_active_at.max(at);
        Box::pin(async { Ok(()) })
    }

    fn disable(&self, id: NetworkAgentId, _at: UtcMillis) -> PortFuture<'_, RepositoryResult<()>> {
        let mut agents = self.agents.lock().unwrap();
        if let Some(agent) = agents.iter_mut().find(|agent| {
            agent.record.id == id && agent.record.status != NetworkAgentStatus::Disabled
        }) {
            agent.record.status = NetworkAgentStatus::Disabled;
            agent.rooms_left = agent.record.agent_instance_id.is_none();
        }
        Box::pin(async { Ok(()) })
    }

    fn take<'a>(
        &'a self,
        bucket: &'a str,
        now: UtcMillis,
        policy: RateWindowPolicy,
    ) -> PortFuture<'a, RepositoryResult<RateWindowDecision>> {
        let window = i64::try_from(policy.window.value()).unwrap();
        let mut windows = self.windows.lock().unwrap();
        let entry = windows.entry(bucket.to_owned()).or_insert((now, 0));
        if now.value() >= entry.0.value() + window {
            *entry = (now, 0);
        }
        let decision = if entry.1 >= policy.limit {
            RateWindowDecision::Limited {
                retry_at: UtcMillis::new(entry.0.value() + window).unwrap(),
            }
        } else {
            entry.1 += 1;
            RateWindowDecision::Allowed
        };
        Box::pin(async move { Ok(decision) })
    }

    fn record_room<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a NetworkAgentRoomRecord,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let mut rooms = self.rooms.lock().unwrap();
        if !rooms
            .iter()
            .any(|(agent, known)| *agent == id && known.matrix_room_id == room.matrix_room_id)
        {
            rooms.push((id, room.clone()));
        }
        Box::pin(async { Ok(()) })
    }

    fn rooms(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentRoomRecord>>> {
        let rooms = self
            .rooms
            .lock()
            .unwrap()
            .iter()
            .filter(|(agent, _)| *agent == id)
            .map(|(_, room)| room.clone())
            .collect();
        Box::pin(async move { Ok(rooms) })
    }

    fn disable_stale(
        &self,
        cutoff: NetworkAgentStaleCutoff,
        _at: UtcMillis,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentId>>> {
        let mut agents = self.agents.lock().unwrap();
        let mut disabled = Vec::new();
        for agent in agents.iter_mut() {
            let stale = match agent.record.status {
                NetworkAgentStatus::Active => agent.record.last_active_at < cutoff.idle_before,
                NetworkAgentStatus::Provisioning => {
                    agent.record.created_at < cutoff.provisioning_before
                }
                NetworkAgentStatus::Disabled => false,
            };
            if stale && disabled.len() < usize::try_from(limit).unwrap() {
                agent.record.status = NetworkAgentStatus::Disabled;
                agent.rooms_left = agent.record.agent_instance_id.is_none();
                disabled.push(agent.record.id);
            }
        }
        Box::pin(async move { Ok(disabled) })
    }

    fn pending_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentRecord>>> {
        let pending = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .filter(|agent| {
                agent.record.status == NetworkAgentStatus::Disabled && !agent.rooms_left
            })
            .take(usize::try_from(limit).unwrap())
            .map(|agent| agent.record.clone())
            .collect();
        Box::pin(async move { Ok(pending) })
    }

    fn mark_rooms_left(
        &self,
        id: NetworkAgentId,
        _at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut agents = self.agents.lock().unwrap();
        if let Some(agent) = agents.iter_mut().find(|agent| {
            agent.record.id == id && agent.record.status == NetworkAgentStatus::Disabled
        }) {
            agent.rooms_left = true;
        }
        Box::pin(async { Ok(()) })
    }
}

/// 把明文包上前缀当作“封存”；测试只关心交给存储的是封存后的值。
struct MarkingSealer;

impl NetworkAgentSecretSealer for MarkingSealer {
    fn seal(
        &self,
        agent: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure> {
        let mut bytes = format!("sealed:{agent}:{}:", kind.as_str()).into_bytes();
        bytes.extend_from_slice(plaintext);
        Ok(SealedSecret {
            key_version: 1,
            bytes,
        })
    }

    fn open(
        &self,
        agent: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure> {
        let prefix = format!("sealed:{agent}:{}:", kind.as_str()).into_bytes();
        sealed
            .bytes
            .strip_prefix(prefix.as_slice())
            .map(<[u8]>::to_vec)
            .ok_or(SecretSealingFailure::Corrupt)
    }
}

#[derive(Default)]
struct CountingKeys(Mutex<u8>);

impl NetworkAgentKeyFactory for CountingKeys {
    fn generate(&self) -> Result<GeneratedSigningKey, SecretGenerationFailure> {
        let mut counter = self.0.lock().unwrap();
        *counter += 1;
        Ok(GeneratedSigningKey {
            encoded_seed: SecretValue::new(format!("seed-{counter}")).unwrap(),
            public_key: [*counter; 32],
        })
    }
}

#[derive(Default)]
struct FakeAgents {
    host_agents: Mutex<Vec<CreateHostAgentForDevice>>,
    instances: Mutex<Vec<RegisterAgentInstance>>,
}

impl AgentManagementUseCases for FakeAgents {
    fn list_agents(
        &self,
        _request: ListAgents,
    ) -> PortFuture<'_, AgentManagementResult<Vec<RegisteredAgent>>> {
        unreachable!("网络 Agent 不列出 Agent")
    }

    fn ensure_default_agent(
        &self,
        _request: EnsureDefaultAgent,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        unreachable!("网络 Agent 没有默认 Agent")
    }

    fn ensure_default_agent_for_device(
        &self,
        _request: EnsureDefaultAgentForDevice,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        unreachable!("网络 Agent 没有默认 Agent")
    }

    fn create_agent(
        &self,
        _request: CreateAgent,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        unreachable!("网络 Agent 按设备建宿主 Agent")
    }

    fn create_host_agent_for_device(
        &self,
        request: CreateHostAgentForDevice,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
        let agent = registered_agent(
            AgentId::from_uuid(request.request_id.as_uuid()),
            &request.display_name,
        );
        self.host_agents.lock().unwrap().push(request);
        Box::pin(async move { Ok(agent) })
    }

    fn register_instance(
        &self,
        request: RegisterAgentInstance,
    ) -> PortFuture<'_, AgentManagementResult<RegisteredAgentInstance>> {
        let agent = registered_agent(request.agent_id, "网络 Agent");
        let instance_id = AgentInstanceId::from_uuid(request.request_id.as_uuid());
        let binding = AdapterBinding::register(
            AdapterBindingId::from_uuid(Uuid::now_v7()),
            request.agent_id,
            request.adapter_type.clone(),
            None,
            request.capability_version.clone(),
        )
        .unwrap();
        let instance = AgentInstance::restore(
            instance_id,
            request.agent_id,
            request.actor.device_id,
            binding.id(),
            request.public_signing_key.clone(),
            AgentMatrixDeviceId::new(format!("AR_{}", instance_id.as_uuid().simple())).unwrap(),
            AgentInstanceStatus::Connecting,
            None,
        )
        .unwrap();
        self.instances.lock().unwrap().push(request);
        Box::pin(async move {
            Ok(RegisteredAgentInstance {
                matrix_session: MatrixSession::new(
                    MatrixSessionMetadata::new(
                        MatrixUserId::new(agent.matrix_user_id.clone()).unwrap(),
                        MatrixDeviceId::new("AR_NETWORK").unwrap(),
                    ),
                    SecretValue::new(MATRIX_TOKEN).unwrap(),
                    None,
                ),
                agent,
                registration: StoredAgentInstanceRegistration { binding, instance },
            })
        })
    }

    fn rotate_instance_matrix_session(
        &self,
        _request: RotateAgentInstanceMatrixSession,
    ) -> PortFuture<'_, AgentManagementResult<RotatedAgentInstanceMatrixSession>> {
        unreachable!("创建时不轮换会话")
    }

    fn change_membership(
        &self,
        _request: ChangeAgentMembership,
    ) -> PortFuture<'_, AgentManagementResult<()>> {
        unreachable!("网络 Agent 不改成员")
    }

    fn delete_agent(&self, _request: DeleteAgent) -> PortFuture<'_, AgentManagementResult<()>> {
        unreachable!("这一步不退役 Agent")
    }
}

/// 按顺序给出进大厅的结果；用完之后都当作进了请求的那间。
#[derive(Default)]
struct ScriptedLobbies {
    script: Mutex<VecDeque<AgentLobbyEntryResult<EnterLobbyOutcome>>>,
    requests: Mutex<Vec<EnterAgentLobby>>,
}

impl ScriptedLobbies {
    fn scripted(outcomes: Vec<AgentLobbyEntryResult<EnterLobbyOutcome>>) -> Self {
        Self {
            script: Mutex::new(outcomes.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn catalogs(&self) -> Vec<RoomCatalogId> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.catalog_id)
            .collect()
    }
}

impl AgentLobbyEntryUseCases for ScriptedLobbies {
    fn enter(
        &self,
        request: EnterAgentLobby,
    ) -> PortFuture<'_, AgentLobbyEntryResult<EnterLobbyOutcome>> {
        let outcome = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(joined(request.catalog_id, request.agent_instance_id)));
        self.requests.lock().unwrap().push(request);
        Box::pin(async move { outcome })
    }
}

struct FakeDirectory(Vec<RoomCatalog>);

impl FakeDirectory {
    fn standard() -> Self {
        Self(vec![
            catalog(1, "rust-night", "Rust 夜谈"),
            catalog(2, "agent-room-global", "Agent Room 大厅"),
            catalog(3, "general", "General"),
        ])
    }
}

impl RoomDirectory for FakeDirectory {
    fn list_public<'a>(
        &'a self,
        _query: &'a RoomDirectoryQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
        let entries = self
            .0
            .iter()
            .map(|catalog| PublicLobbyDirectoryEntry {
                catalog: catalog.clone(),
                active_instance_count: 1,
                online_agent_count: 0,
                activity_score_millis: 0,
            })
            .collect();
        Box::pin(async move { Ok(entries) })
    }

    fn find_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
        let found = self
            .0
            .iter()
            .find(|catalog| catalog.id() == catalog_id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn find_public_observation_room(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>> {
        unreachable!("网络 Agent 不旁观大厅")
    }
}

#[derive(Default)]
struct CountingSecrets(Mutex<u32>);

impl SecretFactory for CountingSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut counter = self.0.lock().unwrap();
        *counter += 1;
        Ok(SecretValue::new(format!("token-{counter}")).unwrap())
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut digest = [0_u8; 32];
        for (index, byte) in value.bytes().enumerate() {
            digest[index % 32] ^= byte.rotate_left(u32::try_from(index % 8).unwrap());
        }
        digest[31] ^= u8::try_from(value.len() % 256).unwrap();
        SecretDigest::from_array(digest)
    }
}

struct TestRuntime {
    now: Mutex<i64>,
    counter: Mutex<u128>,
    pauses: Mutex<Vec<UtcMillis>>,
}

impl TestRuntime {
    fn new() -> Self {
        Self {
            now: Mutex::new(START),
            counter: Mutex::new(0),
            pauses: Mutex::new(Vec::new()),
        }
    }

    fn advance(&self, millis: i64) {
        *self.now.lock().unwrap() += millis;
    }

    fn next(&self) -> Uuid {
        let mut counter = self.counter.lock().unwrap();
        *counter += 1;
        Uuid::from_u128(0x0198_b601_0000_7000_8000_0000_0000_0000 | *counter)
    }
}

impl Clock for TestRuntime {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(*self.now.lock().unwrap()).unwrap()
    }
}

impl NetworkAgentPause for TestRuntime {
    fn until(&self, at: UtcMillis) -> PortFuture<'_, ()> {
        self.pauses.lock().unwrap().push(at);
        Box::pin(async {})
    }
}

impl IdentifierFactory for TestRuntime {
    fn principal_id(&self) -> PrincipalId {
        PrincipalId::from_uuid(self.next())
    }
    fn login_attempt_id(&self) -> LoginAttemptId {
        unreachable!()
    }
    fn web_session_id(&self) -> WebSessionId {
        unreachable!()
    }
    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(self.next())
    }
    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        unreachable!()
    }
    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        unreachable!()
    }
    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        unreachable!()
    }
    fn agent_id(&self) -> AgentId {
        unreachable!()
    }
    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        unreachable!()
    }
    fn adapter_binding_id(&self) -> AdapterBindingId {
        unreachable!()
    }
    fn agent_instance_id(&self) -> AgentInstanceId {
        unreachable!()
    }
    fn room_catalog_id(&self) -> RoomCatalogId {
        unreachable!()
    }
    fn room_instance_id(&self) -> RoomInstanceId {
        unreachable!()
    }
    fn room_reservation_id(&self) -> RoomReservationId {
        unreachable!()
    }
    fn content_id(&self) -> ContentId {
        unreachable!()
    }
    fn handoff_id(&self) -> HandoffId {
        unreachable!()
    }
    fn automation_grant_id(&self) -> AutomationGrantId {
        unreachable!()
    }
    fn outbox_event_id(&self) -> OutboxEventId {
        unreachable!()
    }
}

// ---------- 组装 ----------

struct Harness {
    service: NetworkAgentService,
    store: Arc<MemoryStore>,
    agents: Arc<FakeAgents>,
    lobbies: Arc<ScriptedLobbies>,
    runtime: Arc<TestRuntime>,
}

impl Harness {
    fn new(policy: NetworkAgentPolicy) -> Self {
        Self::with_lobbies(policy, ScriptedLobbies::default())
    }

    fn with_lobbies(policy: NetworkAgentPolicy, lobbies: ScriptedLobbies) -> Self {
        let store = Arc::new(MemoryStore::default());
        let agents = Arc::new(FakeAgents::default());
        let lobbies = Arc::new(lobbies);
        let runtime = Arc::new(TestRuntime::new());
        let service = NetworkAgentService::new(NetworkAgentDependencies {
            store: store.clone(),
            sealer: Arc::new(MarkingSealer),
            keys: Arc::new(CountingKeys::default()),
            agents: agents.clone(),
            lobbies: lobbies.clone(),
            directory: Arc::new(FakeDirectory::standard()),
            secrets: Arc::new(CountingSecrets::default()),
            clock: runtime.clone(),
            identifiers: runtime.clone(),
            pause: runtime.clone(),
            policy,
            matrix_server_name: "matrix.test".to_owned(),
        });
        Self {
            service,
            store,
            agents,
            lobbies,
            runtime,
        }
    }

    fn enabled() -> Self {
        Self::new(NetworkAgentPolicy::default_limits(true))
    }

    fn generous() -> Self {
        Self::new(NetworkAgentPolicy {
            creations_per_source_per_hour: 100,
            creations_per_source_per_day: 100,
            ..NetworkAgentPolicy::default_limits(true)
        })
    }

    async fn create(
        &self,
        name: &str,
        room: Option<&str>,
    ) -> Result<agent_room_application::network_agents::CreatedNetworkAgent, NetworkAgentFailureKind>
    {
        self.create_from(name, room, [1; 32]).await
    }

    async fn create_from(
        &self,
        name: &str,
        room: Option<&str>,
        source_digest: [u8; 32],
    ) -> Result<agent_room_application::network_agents::CreatedNetworkAgent, NetworkAgentFailureKind>
    {
        self.service
            .create(CreateNetworkAgent {
                name: name.to_owned(),
                room: room.map(str::to_owned),
                source_digest,
            })
            .await
            .map_err(|failure| failure.kind())
    }
}

fn catalog(seed: u128, slug: &str, name: &str) -> RoomCatalog {
    RoomCatalog::new(
        catalog_id(seed),
        RoomCatalogFields {
            kind: RoomCatalogKind::PublicLobby,
            slug: Some(RoomSlug::new(slug).unwrap()),
            name: name.to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: None,
            visibility: RoomCatalogVisibility::Public,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        },
    )
    .unwrap()
}

fn catalog_id(seed: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(
        0x0198_b601_0000_7000_8000_0000_0001_0000 | seed,
    ))
}

fn joined(catalog_id: RoomCatalogId, agent_instance_id: AgentInstanceId) -> EnterLobbyOutcome {
    let room_instance_id = RoomInstanceId::from_uuid(Uuid::now_v7());
    EnterLobbyOutcome::Joined {
        reservation: RoomReservation::restore(
            RoomReservationId::from_uuid(Uuid::now_v7()),
            RoomReservationFields {
                catalog_id,
                room_instance_id,
                agent_instance_id,
                reserved_at: UtcMillis::new(START).unwrap(),
                expires_at: UtcMillis::new(START + 60_000).unwrap(),
                state: RoomReservationState::Committed,
                finalized_at: Some(UtcMillis::new(START).unwrap()),
            },
        )
        .unwrap(),
        room: RoomInstance::restore(
            room_instance_id,
            RoomInstanceFields {
                catalog_id,
                matrix_room_id: MatrixRoomReference::new(format!(
                    "!lobby-{}:matrix.test",
                    catalog_id.as_uuid().simple()
                ))
                .unwrap(),
                region: None,
                capacity: RoomCapacity::standard(),
                projected_member_count: 1,
                allocated_slots: 1,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )
        .unwrap(),
        kind: LobbyJoinKind::NewAssignment,
    }
}

fn registered_agent(agent_id: AgentId, display_name: &str) -> RegisteredAgent {
    RegisteredAgent {
        agent: Agent::register(agent_id),
        matrix_user_id: format!("@_agent_{}:matrix.test", agent_id.as_uuid().simple()),
        slug: "network-agent".to_owned(),
        display_name: display_name.to_owned(),
        description: String::new(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: UtcMillis::new(START).unwrap(),
    }
}

fn sealed_text(sealed: &SealedSecret) -> String {
    String::from_utf8(sealed.bytes.clone()).unwrap()
}

// ---------- 用例 ----------

#[tokio::test]
async fn 起名进默认大厅_令牌只返回一次_库里只有摘要和封存的秘密() {
    let harness = Harness::enabled();

    let created = harness.create("  Scout ", None).await.expect("创建成功");

    assert_eq!(created.display_name, "Scout");
    assert_eq!(created.token.expose(), "token-1");
    assert_eq!(created.room.catalog_id, catalog_id(2));
    assert_eq!(created.room.name, "Agent Room 大厅");

    let record = harness.store.only();
    assert_eq!(record.status, NetworkAgentStatus::Active);
    assert_eq!(record.agent_id, Some(created.agent_id));
    let instance_id = record.agent_instance_id.expect("生效时带着实例");
    harness.store.with(created.network_agent_id, |stored| {
        assert_eq!(
            stored.token_digest,
            CountingSecrets::default().digest("token-1")
        );
        assert_eq!(stored.principal.oidc_issuer, NETWORK_AGENT_ISSUER);
        assert_eq!(
            stored.principal.oidc_subject,
            created.network_agent_id.to_string()
        );
        assert_eq!(stored.principal.display_name, "Scout");
        assert!(stored.principal.matrix_user_id.ends_with(":matrix.test"));
        assert_eq!(stored.device_platform, DevicePlatform::Network);
        assert_eq!(stored.device_trust, DeviceTrustState::Verified);
        let kinds: Vec<_> = stored.secrets.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            [
                NetworkAgentSecretKind::DeviceSigningSeed,
                NetworkAgentSecretKind::InstanceSigningSeed,
                NetworkAgentSecretKind::MatrixAccessToken,
            ]
        );
        let texts: Vec<_> = stored
            .secrets
            .iter()
            .map(|(_, sealed)| sealed_text(sealed))
            .collect();
        assert!(texts[0].starts_with("sealed:") && texts[0].ends_with(":seed-1"));
        assert!(texts[1].ends_with(":seed-2"));
        assert!(texts[2].ends_with(&format!(":{MATRIX_TOKEN}")));
    });

    // 以这台网络设备的身份建宿主 Agent、登记实例：请求 ID 都是网络 Agent 的 ID。
    let host = harness.agents.host_agents.lock().unwrap()[0].clone();
    assert_eq!(
        host.request_id.as_uuid(),
        created.network_agent_id.as_uuid()
    );
    assert_eq!(host.display_name, "Scout");
    assert_eq!(host.actor.account.principal.id(), record.principal_id);
    assert_eq!(host.actor.device_id, record.device_id);
    let instance = harness.agents.instances.lock().unwrap()[0].clone();
    assert_eq!(instance.adapter_type, "network");
    assert_eq!(instance.agent_id, created.agent_id);
    assert_eq!(*instance.public_signing_key.as_bytes(), [2; 32]);

    let entered = harness.lobbies.requests.lock().unwrap()[0].clone();
    assert_eq!(entered.agent_id, created.agent_id);
    assert_eq!(entered.agent_instance_id, instance_id);
    assert_eq!(entered.catalog_id, catalog_id(2));

    let me = harness.service.me("token-1").await.expect("令牌有效");
    assert_eq!(me.display_name, "Scout");
    assert_eq!(me.agent_id, created.agent_id);
    // 进过的大厅记在它名下，查看自己时带着房间名。
    assert_eq!(me.rooms, std::slice::from_ref(&created.room));

    // 网关收发时取出的会话：Matrix 令牌是解封后的原文。
    let session = harness.service.session("token-1").await.expect("会话");
    assert_eq!(session.network_agent_id, created.network_agent_id);
    assert_eq!(session.agent_id, created.agent_id);
    assert_eq!(session.agent_instance_id, instance_id);
    assert_eq!(session.display_name, "Scout");
    assert_eq!(session.matrix_access_token.expose(), MATRIX_TOKEN);
    // 签名种子是登记实例时那一把（第二把生成的密钥）的种子。
    assert_eq!(session.instance_signing_seed.expose(), "seed-2");
    assert_eq!(session.principal_id, record.principal_id);
    assert_eq!(
        session.agent_matrix_user_id,
        format!(
            "@_agent_{}:matrix.test",
            created.agent_id.as_uuid().simple()
        )
    );
    assert_eq!(session.rooms.len(), 1);
    assert_eq!(session.rooms[0].catalog_id, catalog_id(2));
    assert_eq!(session.rooms[0].matrix_room_id, created.room.matrix_room_id);
}

#[tokio::test]
async fn 取会话要生效中的令牌_封存的会话打不开时报依赖不可用() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();

    for wrong in ["", "token-2"] {
        assert_eq!(
            harness.service.session(wrong).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Unauthorized
        );
    }

    // 库里的密文被改过：解不开就不给会话，也不当成令牌错误。
    {
        let mut agents = harness.store.agents.lock().unwrap();
        let stored = agents
            .iter_mut()
            .find(|agent| agent.record.id == created.network_agent_id)
            .unwrap();
        for (kind, sealed) in &mut stored.secrets {
            if *kind == NetworkAgentSecretKind::MatrixAccessToken {
                sealed.bytes = b"tampered".to_vec();
            }
        }
    }
    assert_eq!(
        harness
            .service
            .session(created.token.expose())
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::DependencyUnavailable
    );

    harness
        .service
        .disable(created.token.expose())
        .await
        .unwrap();
    assert_eq!(
        harness
            .service
            .session(created.token.expose())
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized
    );
}

#[tokio::test]
async fn 三十天没活动的自动停用_有活动就不算闲置_停用后等着替它离开房间() {
    const DAY: i64 = 24 * 60 * 60 * 1_000;
    let harness = Harness::generous();
    let idle = harness.create("Idle", None).await.unwrap();
    let busy = harness.create("Busy", None).await.unwrap();
    harness.runtime.advance(20 * DAY);
    harness.service.session(busy.token.expose()).await.unwrap();
    harness.runtime.advance(10 * DAY);
    assert_eq!(
        harness.service.disable_stale().await.unwrap(),
        0,
        "刚好三十天还不算"
    );

    harness.runtime.advance(1);
    assert_eq!(harness.service.disable_stale().await.unwrap(), 1);
    assert_eq!(
        harness
            .service
            .session(idle.token.expose())
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized
    );
    harness.service.session(busy.token.expose()).await.unwrap();

    let exits = harness.service.pending_exits(10).await.unwrap();
    let [NetworkAgentPendingExit::Session(session)] = exits.as_slice() else {
        panic!("停用的要等着离开房间：{exits:?}");
    };
    assert_eq!(session.network_agent_id, idle.network_agent_id);
    assert_eq!(session.display_name, "Idle");
    assert_eq!(session.rooms.len(), 1, "带上它进过的大厅");
    harness
        .service
        .mark_rooms_left(idle.network_agent_id)
        .await
        .unwrap();
    assert!(harness.service.pending_exits(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn 停用后会话打不开的_如实告诉清理方() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    {
        let mut agents = harness.store.agents.lock().unwrap();
        let stored = agents
            .iter_mut()
            .find(|agent| agent.record.id == created.network_agent_id)
            .unwrap();
        for (kind, sealed) in &mut stored.secrets {
            if *kind == NetworkAgentSecretKind::InstanceSigningSeed {
                sealed.bytes = b"tampered".to_vec();
            }
        }
    }
    harness
        .service
        .disable(created.token.expose())
        .await
        .unwrap();

    assert_eq!(
        harness.service.pending_exits(10).await.unwrap(),
        [NetworkAgentPendingExit::Unopenable(
            created.network_agent_id
        )]
    );
}

#[tokio::test]
async fn 列出能进的公开大厅_标出默认的那间_总开关关着时回答已关闭() {
    let harness = Harness::enabled();

    let lobbies = harness.service.public_lobbies().await.unwrap();

    assert!(!lobbies.is_empty());
    assert_eq!(
        lobbies.iter().filter(|lobby| lobby.default).count(),
        1,
        "{lobbies:?}"
    );
    let default = lobbies.iter().find(|lobby| lobby.default).unwrap();
    assert_eq!(default.slug.as_deref(), Some("agent-room-global"));

    let disabled = Harness::new(NetworkAgentPolicy::default_limits(false));
    assert_eq!(
        disabled.service.public_lobbies().await.unwrap_err().kind(),
        NetworkAgentFailureKind::Disabled
    );
}

#[tokio::test]
async fn 按名字或短名找公开大厅_忽略大小写_找不到时列出候选() {
    let harness = Harness::generous();

    let exact = harness.create("A", Some("Rust 夜谈")).await.unwrap();
    assert_eq!(exact.room.catalog_id, catalog_id(1));
    let slug = harness.create("B", Some("RUST-NIGHT")).await.unwrap();
    assert_eq!(slug.room.catalog_id, catalog_id(1));
    let lowered = harness.create("C", Some("general")).await.unwrap();
    assert_eq!(lowered.room.catalog_id, catalog_id(3));

    let failure = harness
        .service
        .create(CreateNetworkAgent {
            name: "D".to_owned(),
            room: Some("没有这间".to_owned()),
            source_digest: [1; 32],
        })
        .await
        .expect_err("没有这个大厅");
    assert_eq!(failure.kind(), NetworkAgentFailureKind::RoomNotFound);
    assert_eq!(failure.rooms(), ["Rust 夜谈", "Agent Room 大厅", "General"]);
    // 找不到房间时不占名字、不计次。
    assert_eq!(harness.store.statuses().len(), 3);
}

#[tokio::test]
async fn 同名时自动加序号_不分大小写_二十个之后请换名字() {
    let harness = Harness::generous();

    assert_eq!(
        harness.create("Scout", None).await.unwrap().display_name,
        "Scout"
    );
    assert_eq!(
        harness.create("scout", None).await.unwrap().display_name,
        "scout 2"
    );
    assert_eq!(
        harness.create("SCOUT", None).await.unwrap().display_name,
        "SCOUT 3"
    );
    for number in 4..=20 {
        let created = harness.create("Scout", None).await.unwrap();
        assert_eq!(created.display_name, format!("Scout {number}"));
    }
    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::NameUnavailable)
    );
}

#[tokio::test]
async fn 名字不合规或冒充平台时拒绝() {
    let harness = Harness::enabled();
    for name in ["", "   ", "Agent Room", "ADMIN", "换\n行", &"名".repeat(65)] {
        assert_eq!(
            harness.create(name, None).await.err(),
            Some(NetworkAgentFailureKind::InvalidName),
            "{name:?}"
        );
    }
    assert!(harness.store.statuses().is_empty());
}

#[tokio::test]
async fn 总开关关着时一律回答已关闭且不碰存储() {
    let harness = Harness::new(NetworkAgentPolicy::default_limits(false));

    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::Disabled)
    );
    for token in ["", "token-1"] {
        assert_eq!(
            harness.service.me(token).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Disabled
        );
        assert_eq!(
            harness.service.session(token).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Disabled
        );
        assert_eq!(
            harness.service.disable(token).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Disabled
        );
    }
    assert!(harness.store.statuses().is_empty());
    assert!(harness.store.windows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 同一来源每小时五个_超出时告诉何时再试_换来源或过了窗口就能再建() {
    let harness = Harness::enabled();
    for index in 0..5 {
        harness
            .create_from(&format!("Agent {index}"), None, [7; 32])
            .await
            .expect("前五个都能建");
    }

    let limited = harness
        .service
        .create(CreateNetworkAgent {
            name: "Agent 5".to_owned(),
            room: None,
            source_digest: [7; 32],
        })
        .await
        .expect_err("第六个被限流");
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + HOUR).unwrap())
    );

    harness
        .create_from("Other source", None, [8; 32])
        .await
        .expect("别的来源不受影响");

    harness.runtime.advance(HOUR);
    harness
        .create_from("Agent 5", None, [7; 32])
        .await
        .expect("过了一小时的窗口就能再建");
}

#[tokio::test]
async fn 发言每分钟二十条_每天一千条_超了告诉何时能再发() {
    let harness = Harness::enabled();
    let created = harness.create("Talker", None).await.unwrap();
    let id = created.network_agent_id;
    for _ in 0..20 {
        harness
            .service
            .take_message_quota(id)
            .await
            .expect("每分钟前二十条");
    }
    let limited = harness
        .service
        .take_message_quota(id)
        .await
        .expect_err("第二十一条被限流");
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + 60_000).unwrap())
    );

    // 过了这一分钟又能发；一天累计到一千条为止。
    for minute in 1..50 {
        harness.runtime.advance(60_000);
        for _ in 0..20 {
            harness
                .service
                .take_message_quota(id)
                .await
                .unwrap_or_else(|_| panic!("第 {minute} 分钟"));
        }
    }
    harness.runtime.advance(60_000);
    let daily = harness
        .service
        .take_message_quota(id)
        .await
        .expect_err("一天一千条用完了");
    assert_eq!(daily.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        daily.retry_at(),
        Some(UtcMillis::new(START + 24 * HOUR).unwrap())
    );
}

#[tokio::test]
async fn 总开关关着时不给发言额度() {
    let harness = Harness::new(NetworkAgentPolicy::default_limits(false));
    assert_eq!(
        harness
            .service
            .take_message_quota(NetworkAgentId::from_uuid(Uuid::now_v7()))
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Disabled
    );
}

#[tokio::test]
async fn 同一来源一天最多二十个() {
    let harness = Harness::enabled();
    for hour in 0..4 {
        for index in 0..5 {
            harness
                .create_from(&format!("Agent {hour}-{index}"), None, [9; 32])
                .await
                .expect("每小时五个，四小时共二十个");
        }
        harness.runtime.advance(HOUR);
    }
    let limited = harness
        .service
        .create(CreateNetworkAgent {
            name: "One more".to_owned(),
            room: None,
            source_digest: [9; 32],
        })
        .await
        .expect_err("一天的额度用完了");
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + 24 * HOUR).unwrap())
    );
}

#[tokio::test]
async fn 全站上限到了就请稍后_停用一个就空出名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        max_live_agents: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let first = harness.create("First", None).await.unwrap();
    assert_eq!(
        harness.create("Second", None).await.err(),
        Some(NetworkAgentFailureKind::CapacityReached)
    );

    harness
        .service
        .disable(first.token.expose())
        .await
        .expect("停用成功");
    harness
        .create("Second", None)
        .await
        .expect("空出名额后能建");
}

#[tokio::test]
async fn 令牌不对或已停用就未认证_停用后同名可以再用() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let token = created.token.expose().to_owned();

    for wrong in ["", "token-2", "Bearer token-1"] {
        assert_eq!(
            harness.service.me(wrong).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Unauthorized,
            "{wrong:?}"
        );
    }

    harness.runtime.advance(5_000);
    harness.service.me(&token).await.unwrap();
    assert_eq!(
        harness.store.only().last_active_at,
        UtcMillis::new(START + 5_000).unwrap()
    );

    harness.service.disable(&token).await.expect("停用成功");
    for _ in 0..2 {
        assert_eq!(
            harness.service.me(&token).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Unauthorized
        );
        assert_eq!(
            harness.service.disable(&token).await.unwrap_err().kind(),
            NetworkAgentFailureKind::Unauthorized
        );
    }

    let again = harness.create("Scout", None).await.unwrap();
    assert_eq!(again.display_name, "Scout");
    assert_ne!(again.token.expose(), token);
}

#[tokio::test]
async fn 大厅正在准备房间时按给的时间等_容量变了就换目录再进() {
    let busy_until = UtcMillis::new(START + 2_000).unwrap();
    let harness = Harness::with_lobbies(
        NetworkAgentPolicy::default_limits(true),
        ScriptedLobbies::scripted(vec![
            Ok(EnterLobbyOutcome::ProvisioningBusy {
                retry_at: busy_until,
            }),
            Ok(EnterLobbyOutcome::CapacityChanged {
                catalog_id: catalog_id(3),
            }),
        ]),
    );

    let created = harness.create("Scout", None).await.expect("第三次进成");

    assert_eq!(*harness.runtime.pauses.lock().unwrap(), [busy_until]);
    assert_eq!(
        harness.lobbies.catalogs(),
        [catalog_id(2), catalog_id(2), catalog_id(3)]
    );
    assert_eq!(created.room.catalog_id, catalog_id(3));
    assert_eq!(created.room.name, "General");
}

#[tokio::test]
async fn 进不了大厅时报依赖不可用_停用这条记录并放开名字() {
    let harness = Harness::with_lobbies(
        NetworkAgentPolicy::default_limits(true),
        ScriptedLobbies::scripted(vec![Err(AgentLobbyEntryFailure::NotFound)]),
    );

    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::DependencyUnavailable)
    );
    assert_eq!(
        harness.store.statuses(),
        [("Scout".to_owned(), NetworkAgentStatus::Disabled)]
    );
    assert_eq!(
        harness.service.me("token-1").await.unwrap_err().kind(),
        NetworkAgentFailureKind::Unauthorized
    );

    let retried = harness.create("Scout", None).await.expect("重试成功");
    assert_eq!(retried.display_name, "Scout");
}

#[tokio::test]
async fn 一直进不去时试三次就放弃() {
    let busy = || {
        Ok(EnterLobbyOutcome::ProvisioningBusy {
            retry_at: UtcMillis::new(START + 1_000).unwrap(),
        })
    };
    let harness = Harness::with_lobbies(
        NetworkAgentPolicy::default_limits(true),
        ScriptedLobbies::scripted(vec![busy(), busy(), busy()]),
    );

    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::DependencyUnavailable)
    );
    assert_eq!(harness.lobbies.catalogs().len(), 3);
    assert_eq!(
        harness.store.statuses(),
        [("Scout".to_owned(), NetworkAgentStatus::Disabled)]
    );
}
