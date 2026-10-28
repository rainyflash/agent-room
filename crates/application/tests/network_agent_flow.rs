use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use agent_room_application::{
    agent_lobbies::{
        AgentLobbyEntryFailure, AgentLobbyEntryResult, AgentLobbyEntryUseCases, EnterAgentLobby,
    },
    agents::{
        AgentManagementResult, AgentManagementUseCases, AgentNaming, ChangeAgentMembership,
        CreateAgent, CreateHostAgentForDevice, DeleteAgent, EnsureDefaultAgent,
        EnsureDefaultAgentForDevice, ListAgents, RegisterAgentInstance, RegisteredAgentInstance,
        RotateAgentInstanceMatrixSession, RotatedAgentInstanceMatrixSession,
    },
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentAdmission, NetworkAgentDependencies,
        NetworkAgentFailureKind, NetworkAgentKnockStatus, NetworkAgentPendingExit,
        NetworkAgentPlacement, NetworkAgentPolicy, NetworkAgentRoom, NetworkAgentRoomRequest,
        NetworkAgentService, NetworkAgentTarget, NetworkAgentUseCases,
    },
    persistence::RepositoryResult,
    ports::{
        AgentInstanceManagementRecord, Clock, GeneratedSigningKey, IdentifierFactory,
        MatrixDeviceId, MatrixSession, MatrixSessionMetadata, MatrixUserId, NetworkAgentActivation,
        NetworkAgentActivity, NetworkAgentBeginOutcome, NetworkAgentIdleLobby,
        NetworkAgentKeyFactory, NetworkAgentLobbyAway, NetworkAgentPause, NetworkAgentProvisioning,
        NetworkAgentRecord, NetworkAgentRoomRecord, NetworkAgentSecretKind,
        NetworkAgentSecretSealer, NetworkAgentStaleCutoff, NetworkAgentStore, NetworkSourceWeights,
        PortFuture, PrivateRoomAgentKnockRecord, PrivateRoomAgentMemberRecord,
        PrivateRoomJoinCodeRecord, PrivateRoomSnapshot, PublicLobbyDirectoryEntry,
        PublicLobbyObservationRoom, RateWindowBucket, RateWindowDecision, RateWindowPolicy,
        RegisteredAgent, RoomDirectory, RoomDirectoryQuery, SealedSecret, SecretDigest,
        SecretFactory, SecretGenerationFailure, SecretSealingFailure, SecretValue,
        StoredAgentInstanceRegistration, UniformSourceWeights,
    },
    private_rooms::{
        AdmittedKnock, AgentAccessFailure, AgentAccessFailureKind, AgentAccessResult,
        AgentAccessView, AnswerKnock, ArchivePrivateRoom, ChangePrivateRoomPermissions,
        CreatePrivateRoom, GeneratedJoinCode, GovernPrivateRoomMember, InspectAgentAccess,
        InspectPrivateRoom, InvitePrivateRoomMember, JoinCodeCaller, KnockOnRoom, KnockResult,
        ListPrivateRooms, ListPrivateRoomsForAccount, ManageJoinCode, PrivateRoomActor,
        PrivateRoomAgentAccessUseCases, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, PrivateRoomMembershipAction, PrivateRoomOrigin, PrivateRoomResult,
        PrivateRoomUseCases, RedeemJoinCode, RedeemedRoom, RemoveAgentMember, RenamePrivateRoom,
        ResolveJoinCode, TransferPrivateRoomOwnership,
    },
    rooms::{EnterLobbyOutcome, LobbyJoinKind},
};
use agent_room_domain::{
    agent_names::{AgentNameTag, untagged_name},
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
    join_codes::{PrivateRoomAgentKnockStatus, PrivateRoomAgentMemberStatus, PrivateRoomJoinCode},
    network_agents::{NETWORK_AGENT_ISSUER, NetworkAgentStatus},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
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
/// 对得上的 Agent 口令。
const CODE: &str = "ABCD-EFGH-JKLM";
const PRIVATE_ROOM: &str = "!private:matrix.test";

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
    /// 离开房间以后删了钥匙。
    keys_deleted: bool,
}

#[derive(Default)]
struct MemoryStore {
    agents: Mutex<Vec<StoredAgent>>,
    windows: Mutex<HashMap<String, (UtcMillis, u32)>>,
    rooms: Mutex<Vec<(NetworkAgentId, NetworkAgentRoomRecord)>>,
    /// 全站网络 Agent 建的、没关的私人房间：测试直接设。
    owned_private_rooms: Mutex<u64>,
    /// 网络 Agent 当房主的房间：测试直接设。
    network_owned: Mutex<HashSet<RoomCatalogId>>,
    /// 闲置时请出、还没回去的公开大厅。
    away: Mutex<Vec<(NetworkAgentId, NetworkAgentLobbyAway)>>,
    /// 接下来这么多次写入都说名字被占了：测试直接设。
    names_taken: Mutex<usize>,
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

    /// 名字去掉了编号，编号由 [`assert_tagged`] 查。
    fn statuses(&self) -> Vec<(String, NetworkAgentStatus)> {
        self.agents
            .lock()
            .unwrap()
            .iter()
            .map(|agent| {
                (
                    untagged_name(&agent.record.display_name).to_owned(),
                    agent.record.status,
                )
            })
            .collect()
    }
}

/// 显示的名字是要的名字加上 ` #` 和四位数字。
#[track_caller]
fn assert_tagged(shown: &str, base: &str) {
    let tag = shown
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix(" #"))
        .unwrap_or_else(|| panic!("{shown} 应当是 {base} 加编号"));
    assert!(
        tag.len() == 4 && tag.bytes().all(|byte| byte.is_ascii_digit()),
        "{shown}"
    );
}

impl NetworkAgentStore for MemoryStore {
    fn begin(
        &self,
        provisioning: &NetworkAgentProvisioning,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentBeginOutcome>> {
        let mut agents = self.agents.lock().unwrap();
        let mut forced = self.names_taken.lock().unwrap();
        let taken = *forced > 0
            || agents.iter().any(|agent| {
                agent.record.status != NetworkAgentStatus::Disabled
                    && agent.record.display_name.to_lowercase()
                        == provisioning.display_name.to_lowercase()
            });
        *forced = forced.saturating_sub(1);
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
                    matrix_device_id: None,
                    display_name: provisioning.display_name.clone(),
                    status: NetworkAgentStatus::Provisioning,
                    created_at: provisioning.created_at,
                    last_active_at: provisioning.created_at,
                    encrypted_since: None,
                },
                token_digest: provisioning.token_digest,
                principal: provisioning.principal.clone(),
                device_platform: provisioning.device.platform(),
                device_trust: provisioning.device.trust_state(),
                secrets: provisioning.secrets.clone(),
                rooms_left: false,
                keys_deleted: false,
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

    fn find(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<NetworkAgentRecord>>> {
        let found = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .find(|agent| agent.record.id == id)
            .map(|agent| agent.record.clone());
        Box::pin(async move { Ok(found) })
    }

    fn find_by_agent(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<NetworkAgentRecord>>> {
        let found = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .find(|agent| agent.record.agent_id == Some(agent_id))
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

    fn count_live_in<'a>(
        &'a self,
        catalog_ids: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<u64>> {
        let rooms = self.rooms.lock().unwrap().clone();
        let count = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .filter(|agent| agent.record.status != NetworkAgentStatus::Disabled)
            .filter(|agent| {
                rooms.iter().any(|(id, room)| {
                    *id == agent.record.id && catalog_ids.contains(&room.catalog_id)
                })
            })
            .count();
        Box::pin(async move { Ok(u64::try_from(count).unwrap()) })
    }

    fn count_owned_private_rooms(&self) -> PortFuture<'_, RepositoryResult<u64>> {
        let count = *self.owned_private_rooms.lock().unwrap();
        Box::pin(async move { Ok(count) })
    }

    fn owned_by_network_agent(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let owned = self.network_owned.lock().unwrap().contains(&catalog_id);
        Box::pin(async move { Ok(owned) })
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

    fn take_all<'a>(
        &'a self,
        buckets: &'a [RateWindowBucket],
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RateWindowDecision>> {
        let mut windows = self.windows.lock().unwrap();
        let mut limited: Option<UtcMillis> = None;
        for bucket in buckets {
            let window = i64::try_from(bucket.policy.window.value()).unwrap();
            let entry = windows.get(&bucket.bucket).copied().unwrap_or((now, 0));
            if now.value() < entry.0.value() + window && entry.1 >= bucket.policy.limit {
                let retry_at = UtcMillis::new(entry.0.value() + window).unwrap();
                limited = Some(limited.map_or(retry_at, |known| known.max(retry_at)));
            }
        }
        if let Some(retry_at) = limited {
            return Box::pin(async move { Ok(RateWindowDecision::Limited { retry_at }) });
        }
        for bucket in buckets {
            let window = i64::try_from(bucket.policy.window.value()).unwrap();
            let entry = windows.entry(bucket.bucket.clone()).or_insert((now, 0));
            if now.value() >= entry.0.value() + window {
                *entry = (now, 0);
            }
            entry.1 += 1;
        }
        Box::pin(async { Ok(RateWindowDecision::Allowed) })
    }

    fn give_back<'a>(
        &'a self,
        buckets: &'a [String],
        taken_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let mut windows = self.windows.lock().unwrap();
        for bucket in buckets {
            if let Some(entry) = windows.get_mut(bucket)
                && entry.0 <= taken_at
            {
                entry.1 = entry.1.saturating_sub(1);
            }
        }
        Box::pin(async { Ok(()) })
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
        self.away
            .lock()
            .unwrap()
            .retain(|(agent, away)| !(*agent == id && away.catalog_id == room.catalog_id));
        Box::pin(async { Ok(()) })
    }

    /// 目录里的三间公开大厅（`FakeDirectory::standard`）；别的都是私人房间。
    fn idle_in_lobbies(
        &self,
        idle_before: UtcMillis,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentIdleLobby>>> {
        let lobbies = [1, 2, 3].map(catalog_id);
        let rooms = self.rooms.lock().unwrap().clone();
        let idle = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .filter(|agent| {
                agent.record.status == NetworkAgentStatus::Active
                    && agent.record.last_active_at < idle_before
            })
            .flat_map(|agent| {
                rooms
                    .iter()
                    .filter(|(id, room)| {
                        *id == agent.record.id && lobbies.contains(&room.catalog_id)
                    })
                    .map(|(id, room)| NetworkAgentIdleLobby {
                        id: *id,
                        room: room.clone(),
                    })
            })
            .take(usize::try_from(limit).unwrap())
            .collect();
        Box::pin(async move { Ok(idle) })
    }

    fn mark_lobby_away<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a NetworkAgentRoomRecord,
        at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let mut rooms = self.rooms.lock().unwrap();
        let before = rooms.len();
        rooms.retain(|(agent, known)| {
            !(*agent == id
                && known.matrix_room_id == room.matrix_room_id
                && known.joined_at <= room.joined_at)
        });
        if rooms.len() < before {
            let mut away = self.away.lock().unwrap();
            away.retain(|(agent, known)| !(*agent == id && known.catalog_id == room.catalog_id));
            away.push((
                id,
                NetworkAgentLobbyAway {
                    catalog_id: room.catalog_id,
                    matrix_room_id: room.matrix_room_id.clone(),
                    left_at: at,
                },
            ));
        }
        Box::pin(async { Ok(()) })
    }

    fn lobbies_away(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentLobbyAway>>> {
        let away = self
            .away
            .lock()
            .unwrap()
            .iter()
            .filter(|(agent, _)| *agent == id)
            .map(|(_, lobby)| lobby.clone())
            .collect();
        Box::pin(async move { Ok(away) })
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

    fn put_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        sealed: &SealedSecret,
        _at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut agents = self.agents.lock().unwrap();
        let agent = agents
            .iter_mut()
            .find(|agent| agent.record.id == id)
            .unwrap();
        agent.secrets.retain(|(stored, _)| *stored != kind);
        agent.secrets.push((kind, sealed.clone()));
        Box::pin(async { Ok(()) })
    }

    fn mark_encrypted(
        &self,
        id: NetworkAgentId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<UtcMillis>> {
        let mut agents = self.agents.lock().unwrap();
        let agent = agents
            .iter_mut()
            .find(|agent| agent.record.id == id)
            .unwrap();
        let since = *agent.record.encrypted_since.get_or_insert(at);
        Box::pin(async move { Ok(since) })
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

    fn pending_key_deletions(
        &self,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentId>>> {
        let pending = self
            .agents
            .lock()
            .unwrap()
            .iter()
            .filter(|agent| {
                agent.record.status == NetworkAgentStatus::Disabled
                    && agent.rooms_left
                    && !agent.keys_deleted
            })
            .take(usize::try_from(limit).unwrap())
            .map(|agent| agent.record.id)
            .collect();
        Box::pin(async move { Ok(pending) })
    }

    fn delete_keys(
        &self,
        id: NetworkAgentId,
        _at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut agents = self.agents.lock().unwrap();
        if let Some(agent) = agents.iter_mut().find(|agent| {
            agent.record.id == id
                && agent.record.status == NetworkAgentStatus::Disabled
                && agent.rooms_left
        }) {
            agent.secrets.clear();
            agent.keys_deleted = true;
            let simple = id.as_uuid().simple().to_string();
            self.windows
                .lock()
                .unwrap()
                .retain(|bucket, _| !bucket.starts_with("send:") || !bucket.ends_with(&simple));
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
    rotations: Mutex<Vec<RotateAgentInstanceMatrixSession>>,
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
        unreachable!("网络 Agent 重建加密存储时换设备，不沿用旧设备")
    }

    /// 加密存储重建时换一台设备：新的设备 ID，新的访问令牌。
    fn replace_instance_matrix_device(
        &self,
        request: RotateAgentInstanceMatrixSession,
    ) -> PortFuture<'_, AgentManagementResult<RotatedAgentInstanceMatrixSession>> {
        let registered = self
            .instances
            .lock()
            .unwrap()
            .iter()
            .find(|instance| {
                AgentInstanceId::from_uuid(instance.request_id.as_uuid()) == request.instance_id
            })
            .cloned()
            .expect("换会话的实例登记过");
        let agent = registered_agent(registered.agent_id, "网络 Agent");
        let instance = AgentInstance::restore(
            request.instance_id,
            registered.agent_id,
            request.actor.device_id,
            AdapterBindingId::from_uuid(Uuid::now_v7()),
            registered.public_signing_key.clone(),
            AgentMatrixDeviceId::new(format!("AR_{}", request.instance_id.as_uuid().simple()))
                .unwrap(),
            AgentInstanceStatus::Connecting,
            None,
        )
        .unwrap();
        let rotations = {
            let mut rotations = self.rotations.lock().unwrap();
            rotations.push(request);
            rotations.len()
        };
        Box::pin(async move {
            Ok(RotatedAgentInstanceMatrixSession {
                instance: AgentInstanceManagementRecord {
                    instance,
                    agent_matrix_user_id: agent.matrix_user_id.clone(),
                    agent_display_name: agent.display_name.clone(),
                    agent_avatar_content_id: None,
                    adapter_type: "network".to_owned(),
                    capability_version: "1.0".to_owned(),
                    device_label: "Agent Room 网络 Agent".to_owned(),
                    device_platform: DevicePlatform::Network,
                    device_trust_state: DeviceTrustState::Verified,
                    created_at: UtcMillis::new(START).unwrap(),
                    last_seen_at: None,
                    revoked_at: None,
                    matrix_device_revoked_at: None,
                },
                matrix_session: MatrixSession::new(
                    MatrixSessionMetadata::new(
                        MatrixUserId::new(agent.matrix_user_id).unwrap(),
                        MatrixDeviceId::new(format!("AR_NETWORK_{rotations}")).unwrap(),
                    ),
                    SecretValue::new(format!("{MATRIX_TOKEN}-rotated-{rotations}")).unwrap(),
                    None,
                ),
            })
        })
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
        let outcome = self.script.lock().unwrap().pop_front().unwrap_or_else(|| {
            Ok(joined_in(
                request.catalog_id,
                request.agent_instance_id,
                request.target_room.clone(),
            ))
        });
        self.requests.lock().unwrap().push(request);
        Box::pin(async move { outcome })
    }
}

/// 私人房间的口令：`CODE` 对得上；`REMOVED` 是被移出后拿来的旧口令（能看不能进）；
/// `LIMITED` 表示这个来源猜错到了上限；其余都不对。
/// 敲门：只有 `catalog_id(9)` 是私人房间；`members` 里的已经是 Agent 成员，`declined` 里的没让进过。
#[derive(Default)]
struct FakeAccess {
    resolved: Mutex<Vec<(JoinCodeCaller, String)>>,
    redeemed: Mutex<Vec<(AgentId, DeviceId, String)>>,
    knocked: Mutex<Vec<(AgentId, DeviceId, RoomCatalogId)>>,
    knocks: Mutex<Vec<PrivateRoomAgentKnockRecord>>,
    members: Mutex<HashSet<AgentId>>,
    declined: Mutex<HashSet<AgentId>>,
    /// 网络 Agent 自己建的房间的口令；看不到的记在 `hidden`。
    codes: Mutex<HashMap<RoomCatalogId, PrivateRoomJoinCode>>,
    hidden: Mutex<HashSet<RoomCatalogId>>,
    /// 管房间的是谁（合成主体）、管的哪间、做了什么（生成、看、停用口令，回答敲门，移出 Agent）。
    managed: Mutex<Vec<(PrincipalId, RoomCatalogId, &'static str)>>,
    /// 移出过的 Agent 成员：再移一次也成功，和真的一样。
    removed: Mutex<HashSet<AgentId>>,
}

impl FakeAccess {
    fn generated_codes(&self) -> usize {
        self.managed
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, _, action)| *action == "generate")
            .count()
    }

    fn manage(&self, actor: &PrivateRoomActor, catalog_id: RoomCatalogId, action: &'static str) {
        assert!(
            matches!(actor, PrivateRoomActor::NetworkAgent(_)),
            "只有网络 Agent 管自己建的房间时才来"
        );
        self.managed
            .lock()
            .unwrap()
            .push((actor.principal_id(), catalog_id, action));
    }

    /// 回答敲门：只认在等的。
    fn waiting_knock(
        &self,
        request: &AnswerKnock,
    ) -> AgentAccessResult<PrivateRoomAgentKnockRecord> {
        self.knocks
            .lock()
            .unwrap()
            .iter()
            .find(|knock| {
                knock.catalog_id == request.catalog_id
                    && knock.agent_id == request.agent_id
                    && knock.status == PrivateRoomAgentKnockStatus::Waiting
            })
            .cloned()
            .ok_or_else(|| AgentAccessFailure::new("test.answer", AgentAccessFailureKind::NotFound))
    }

    fn decide(&self, request: &AnswerKnock, status: PrivateRoomAgentKnockStatus) {
        for knock in self.knocks.lock().unwrap().iter_mut() {
            if knock.catalog_id == request.catalog_id
                && knock.agent_id == request.agent_id
                && knock.status == PrivateRoomAgentKnockStatus::Waiting
            {
                knock.status = status;
                knock.decided_at = Some(UtcMillis::new(START).unwrap());
            }
        }
    }
}

/// 有个 Agent 在敲这个房间的门。
fn waiting_knock_on(
    catalog_id: RoomCatalogId,
    agent_id: AgentId,
    name: &str,
) -> PrivateRoomAgentKnockRecord {
    PrivateRoomAgentKnockRecord {
        catalog_id,
        agent_id,
        display_name: name.to_owned(),
        status: PrivateRoomAgentKnockStatus::Waiting,
        knocked_at: UtcMillis::new(START).unwrap(),
        expires_at: UtcMillis::new(START + HOUR).unwrap(),
        decided_at: None,
    }
}

fn generated_code(catalog_id: RoomCatalogId, code: &PrivateRoomJoinCode) -> GeneratedJoinCode {
    GeneratedJoinCode {
        code: code.clone(),
        record: PrivateRoomJoinCodeRecord {
            catalog_id,
            permissions: PrivateRoomPermissions::INVITED_BY_AGENT,
            created_by: PrincipalId::from_uuid(Uuid::nil()),
            created_at: UtcMillis::new(START).unwrap(),
        },
    }
}

impl PrivateRoomAgentAccessUseCases for FakeAccess {
    fn inspect(
        &self,
        _request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<AgentAccessView>> {
        unreachable!("网络 Agent 不管理口令")
    }

    /// 只有网络 Agent 管自己建的房间时才来：按生成的次数给不同的口令。
    fn generate_code(
        &self,
        request: ManageJoinCode,
    ) -> PortFuture<'_, AgentAccessResult<GeneratedJoinCode>> {
        assert!(matches!(request.actor, PrivateRoomActor::NetworkAgent(_)));
        self.managed.lock().unwrap().push((
            request.actor.principal_id(),
            request.catalog_id,
            "generate",
        ));
        let entropy = u8::try_from(self.generated_codes()).unwrap();
        let code = PrivateRoomJoinCode::from_entropy([entropy; 8]);
        self.codes
            .lock()
            .unwrap()
            .insert(request.catalog_id, code.clone());
        self.hidden.lock().unwrap().remove(&request.catalog_id);
        let generated = generated_code(request.catalog_id, &code);
        Box::pin(async move { Ok(generated) })
    }

    fn reveal_code(
        &self,
        request: ManageJoinCode,
    ) -> PortFuture<'_, AgentAccessResult<GeneratedJoinCode>> {
        assert!(matches!(request.actor, PrivateRoomActor::NetworkAgent(_)));
        self.managed.lock().unwrap().push((
            request.actor.principal_id(),
            request.catalog_id,
            "reveal",
        ));
        let result = if self.hidden.lock().unwrap().contains(&request.catalog_id) {
            Err(AgentAccessFailure::new(
                "test.reveal",
                AgentAccessFailureKind::NotRevealable,
            ))
        } else {
            self.codes
                .lock()
                .unwrap()
                .get(&request.catalog_id)
                .map(|code| generated_code(request.catalog_id, code))
                .ok_or_else(|| {
                    AgentAccessFailure::new("test.reveal", AgentAccessFailureKind::NotFound)
                })
        };
        Box::pin(async move { result })
    }

    fn disable_code(&self, request: ManageJoinCode) -> PortFuture<'_, AgentAccessResult<()>> {
        self.manage(&request.actor, request.catalog_id, "disable");
        self.codes.lock().unwrap().remove(&request.catalog_id);
        self.hidden.lock().unwrap().remove(&request.catalog_id);
        Box::pin(async { Ok(()) })
    }

    /// 只移得出凭口令或敲门进来的（`members`）；移出过的再移一次也成功。
    fn remove_agent(&self, request: RemoveAgentMember) -> PortFuture<'_, AgentAccessResult<()>> {
        self.manage(&request.actor, request.catalog_id, "remove");
        let result = if self.members.lock().unwrap().remove(&request.agent_id) {
            self.removed.lock().unwrap().insert(request.agent_id);
            Ok(())
        } else if self.removed.lock().unwrap().contains(&request.agent_id) {
            Ok(())
        } else {
            Err(AgentAccessFailure::new(
                "test.remove",
                AgentAccessFailureKind::NotFound,
            ))
        };
        Box::pin(async move { result })
    }

    fn resolve(&self, request: ResolveJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        const OPERATION: &str = "test.resolve";
        let result = match request.code.as_str() {
            CODE | "REMOVED" => Ok(private_redeemed()),
            "LIMITED" => Err(AgentAccessFailure::rate_limited(
                OPERATION,
                UtcMillis::new(START + HOUR).unwrap(),
            )),
            _ => Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::NotFound,
            )),
        };
        self.resolved
            .lock()
            .unwrap()
            .push((request.caller, request.code));
        Box::pin(async move { result })
    }

    fn redeem(&self, request: RedeemJoinCode) -> PortFuture<'_, AgentAccessResult<RedeemedRoom>> {
        const OPERATION: &str = "test.redeem";
        let result = match request.code.as_str() {
            CODE => Ok(private_redeemed()),
            "REMOVED" => Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::Forbidden,
            )),
            _ => Err(AgentAccessFailure::new(
                OPERATION,
                AgentAccessFailureKind::NotFound,
            )),
        };
        self.redeemed.lock().unwrap().push((
            request.agent_id,
            request.actor.device_id,
            request.code,
        ));
        Box::pin(async move { result })
    }

    fn knockable(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, AgentAccessResult<()>> {
        let result = if catalog_id == private_catalog() {
            Ok(())
        } else {
            Err(AgentAccessFailure::new(
                "test.knockable",
                AgentAccessFailureKind::NotFound,
            ))
        };
        Box::pin(async move { result })
    }

    fn knock(&self, request: KnockOnRoom) -> PortFuture<'_, AgentAccessResult<KnockResult>> {
        self.knocked.lock().unwrap().push((
            request.agent_id,
            request.actor.device_id,
            request.catalog_id,
        ));
        let result = if request.catalog_id != private_catalog() {
            Err(AgentAccessFailure::new(
                "test.knock",
                AgentAccessFailureKind::NotFound,
            ))
        } else if self.members.lock().unwrap().contains(&request.agent_id) {
            Ok(KnockResult::Member(private_redeemed()))
        } else {
            let declined = self.declined.lock().unwrap().contains(&request.agent_id);
            let record = PrivateRoomAgentKnockRecord {
                catalog_id: request.catalog_id,
                agent_id: request.agent_id,
                display_name: "Scout".to_owned(),
                status: if declined {
                    PrivateRoomAgentKnockStatus::Declined
                } else {
                    PrivateRoomAgentKnockStatus::Waiting
                },
                knocked_at: UtcMillis::new(START).unwrap(),
                expires_at: UtcMillis::new(START + HOUR).unwrap(),
                decided_at: declined.then(|| UtcMillis::new(START).unwrap()),
            };
            let mut knocks = self.knocks.lock().unwrap();
            knocks.retain(|knock| knock.agent_id != request.agent_id);
            knocks.push(record.clone());
            Ok(if declined {
                KnockResult::Declined(record)
            } else {
                KnockResult::Waiting(record)
            })
        };
        Box::pin(async move { result })
    }

    fn knocks_of(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>>> {
        let found = self
            .knocks
            .lock()
            .unwrap()
            .iter()
            .filter(|knock| knock.agent_id == agent_id)
            .cloned()
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn waiting_knocks(
        &self,
        request: InspectAgentAccess,
    ) -> PortFuture<'_, AgentAccessResult<Vec<PrivateRoomAgentKnockRecord>>> {
        self.manage(&request.actor, request.catalog_id, "knocks");
        let waiting = self
            .knocks
            .lock()
            .unwrap()
            .iter()
            .filter(|knock| {
                knock.catalog_id == request.catalog_id
                    && knock.status == PrivateRoomAgentKnockStatus::Waiting
            })
            .cloned()
            .collect();
        Box::pin(async move { Ok(waiting) })
    }

    /// 放行只记成员，敲门还在等，进去了再 `complete_knock`，和真的一样。
    fn admit_knock(
        &self,
        request: AnswerKnock,
    ) -> PortFuture<'_, AgentAccessResult<AdmittedKnock>> {
        self.manage(&request.actor, request.catalog_id, "admit");
        let result = self.waiting_knock(&request).map(|knock| AdmittedKnock {
            room: RedeemedRoom {
                catalog_id: request.catalog_id,
                matrix_room_id: owned_matrix_room(request.catalog_id),
                name: "Owned".to_owned(),
            },
            agent: PrivateRoomAgentMemberRecord {
                catalog_id: request.catalog_id,
                agent_id: request.agent_id,
                display_name: knock.display_name,
                matrix_user_id: MatrixUserId::new("@knocker:matrix.test".to_owned()).unwrap(),
                owner_display_name: None,
                status: PrivateRoomAgentMemberStatus::Joined,
                permissions: PrivateRoomPermissions::AGENT_MEMBER,
                joined_at: UtcMillis::new(START).unwrap(),
                status_changed_at: UtcMillis::new(START).unwrap(),
            },
        });
        Box::pin(async move { result })
    }

    fn complete_knock(&self, request: AnswerKnock) -> PortFuture<'_, AgentAccessResult<()>> {
        self.manage(&request.actor, request.catalog_id, "complete");
        let result = self
            .waiting_knock(&request)
            .map(|_| self.decide(&request, PrivateRoomAgentKnockStatus::Admitted));
        Box::pin(async move { result })
    }

    /// 原本就不在等也算成功，和真的一样。
    fn decline_knock(&self, request: AnswerKnock) -> PortFuture<'_, AgentAccessResult<()>> {
        self.manage(&request.actor, request.catalog_id, "decline");
        self.decide(&request, PrivateRoomAgentKnockStatus::Declined);
        Box::pin(async { Ok(()) })
    }
}

fn private_redeemed() -> RedeemedRoom {
    RedeemedRoom {
        catalog_id: catalog_id(9),
        matrix_room_id: MatrixRoomReference::new(PRIVATE_ROOM.to_owned()).unwrap(),
        name: "项目室".to_owned(),
    }
}

/// 网络 Agent 自己建、自己关的私人房间：房间按领域规则建，谁能进、谁能关都照领域规则判断。
#[derive(Default)]
struct FakeRooms {
    rooms: Mutex<Vec<PrivateRoomSnapshot>>,
    created: Mutex<Vec<CreatePrivateRoom>>,
    archived: Mutex<Vec<(PrincipalId, RoomCatalogId)>>,
    invited: Mutex<Vec<InvitePrivateRoomMember>>,
}

/// 没有这个账户：邀请时按主体找不到 Matrix 账号。
fn no_account() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(404))
}

impl FakeRooms {
    /// 别人当房主的一间私人房间。
    fn seed_other(&self, owner: PrincipalId) -> RoomCatalogId {
        let catalog_id = catalog_id(77);
        self.rooms.lock().unwrap().push(owned_snapshot(
            catalog_id,
            owner,
            None,
            "别人的房间",
            false,
        ));
        catalog_id
    }
}

/// 一间私人房间：建房的 Agent 给了就是 Agent 建的。
fn owned_snapshot(
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    agent: Option<AgentId>,
    name: &str,
    archived: bool,
) -> PrivateRoomSnapshot {
    let mut room = agent.map_or_else(
        || PrivateRoom::create(catalog_id, owner),
        |agent| PrivateRoom::create_by_agent(catalog_id, owner, agent),
    );
    let (catalog_status, instance_state) = if archived {
        room.archive(owner).unwrap();
        (RoomCatalogStatus::Archived, RoomInstanceState::Archived)
    } else {
        (RoomCatalogStatus::Active, RoomInstanceState::Active)
    };
    let catalog = RoomCatalog::new(
        catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: name.to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: catalog_status,
        },
    )
    .unwrap();
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(catalog_id.as_uuid()),
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: owned_matrix_room(catalog_id),
            region: None,
            capacity: RoomCapacity::new(8, 16).unwrap(),
            projected_member_count: 1,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: instance_state,
        },
    )
    .unwrap();
    PrivateRoomSnapshot::new(catalog, instance, room).unwrap()
}

fn owned_matrix_room(catalog_id: RoomCatalogId) -> MatrixRoomReference {
    MatrixRoomReference::new(format!(
        "!owned{}:matrix.test",
        catalog_id.as_uuid().simple()
    ))
    .unwrap()
}

fn room_failure(kind: PrivateRoomFailureKind) -> PrivateRoomFailure {
    PrivateRoomFailure::new(
        "test.private_room",
        PrivateRoomFailureStage::Validation,
        kind,
    )
}

impl PrivateRoomUseCases for FakeRooms {
    fn create(
        &self,
        request: CreatePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        assert!(matches!(request.actor, PrivateRoomActor::NetworkAgent(_)));
        let snapshot = owned_snapshot(
            request.catalog_id,
            request.actor.principal_id(),
            request.created_by_agent,
            &request.name,
            false,
        );
        self.rooms.lock().unwrap().push(snapshot.clone());
        self.created.lock().unwrap().push(request);
        Box::pin(async move { Ok(snapshot) })
    }

    fn inspect(
        &self,
        _request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不按网页登录查看房间")
    }

    fn origin(
        &self,
        _request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomOrigin>> {
        unreachable!("网络 Agent 不问房间是谁建的")
    }

    fn list(
        &self,
        _request: ListPrivateRooms,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        unreachable!("网络 Agent 不按网页登录列房间")
    }

    /// 和真的一样只列它的 Agent 进得去的：没关、它是已加入能发言的成员。
    fn list_for_account(
        &self,
        request: ListPrivateRoomsForAccount,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        let rooms = self
            .rooms
            .lock()
            .unwrap()
            .iter()
            .filter(|snapshot| snapshot.room().admits_agent_of(request.principal_id))
            .cloned()
            .collect();
        Box::pin(async move { Ok(rooms) })
    }

    /// 网络 Agent 在自己建的房间里邀请人；`no_account()` 按主体找不到 Matrix 账号。
    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        assert!(matches!(request.actor, PrivateRoomActor::NetworkAgent(_)));
        let result = if request.target_principal_id == no_account() {
            Err(PrivateRoomFailure::new(
                "test.invite",
                PrivateRoomFailureStage::Directory,
                PrivateRoomFailureKind::NotFound,
            ))
        } else {
            self.rooms
                .lock()
                .unwrap()
                .iter()
                .find(|snapshot| snapshot.catalog().id() == request.catalog_id)
                .cloned()
                .ok_or_else(|| room_failure(PrivateRoomFailureKind::NotFound))
        };
        self.invited.lock().unwrap().push(request);
        Box::pin(async move { result })
    }

    fn accept(
        &self,
        _request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不接受邀请")
    }

    fn decline(
        &self,
        _request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不拒绝邀请")
    }

    fn leave(
        &self,
        _request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不按成员离开房间")
    }

    fn remove(
        &self,
        _request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不移出人")
    }

    fn ban(
        &self,
        _request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不封禁人")
    }

    fn unban(
        &self,
        _request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不解封人")
    }

    fn update_permissions(
        &self,
        _request: ChangePrivateRoomPermissions,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不改人的权限")
    }

    fn transfer_ownership(
        &self,
        _request: TransferPrivateRoomOwnership,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不转让房间")
    }

    /// 只认房主，关了的再关一次照样成功（领域规则）。
    fn archive(
        &self,
        request: ArchivePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        let actor = request.actor.principal_id();
        let mut rooms = self.rooms.lock().unwrap();
        let result = match rooms
            .iter_mut()
            .find(|snapshot| snapshot.catalog().id() == request.catalog_id)
        {
            None => Err(room_failure(PrivateRoomFailureKind::NotFound)),
            Some(snapshot) if snapshot.room().clone().archive(actor).is_err() => {
                Err(room_failure(PrivateRoomFailureKind::Forbidden))
            }
            Some(snapshot) => {
                *snapshot = owned_snapshot(
                    request.catalog_id,
                    actor,
                    snapshot.room().created_by_agent(),
                    snapshot.catalog().name(),
                    true,
                );
                self.archived
                    .lock()
                    .unwrap()
                    .push((actor, request.catalog_id));
                Ok(snapshot.clone())
            }
        };
        Box::pin(async move { result })
    }

    fn rename(
        &self,
        _request: RenamePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        unreachable!("网络 Agent 不改房间名")
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
        RoomCatalogId::from_uuid(self.next())
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

/// 把一个来源当成共享出口：它的名额乘上分量，别的来源都算一个。
struct SharedSource {
    source: [u8; 32],
    weight: u32,
}

impl NetworkSourceWeights for SharedSource {
    fn weight(&self, source: &[u8; 32]) -> u32 {
        if *source == self.source {
            self.weight
        } else {
            1
        }
    }
}

/// 记下凭令牌认出来的网络 Agent。
#[derive(Default)]
struct RecordingActivity(Mutex<Vec<NetworkAgentId>>);

impl RecordingActivity {
    fn seen(&self) -> Vec<NetworkAgentId> {
        self.0.lock().unwrap().clone()
    }
}

impl NetworkAgentActivity for RecordingActivity {
    fn authenticated(&self, agent: NetworkAgentId) {
        self.0.lock().unwrap().push(agent);
    }
}

struct Harness {
    service: NetworkAgentService,
    store: Arc<MemoryStore>,
    activity: Arc<RecordingActivity>,
    agents: Arc<FakeAgents>,
    lobbies: Arc<ScriptedLobbies>,
    access: Arc<FakeAccess>,
    rooms: Arc<FakeRooms>,
    runtime: Arc<TestRuntime>,
}

impl Harness {
    fn new(policy: NetworkAgentPolicy) -> Self {
        Self::with_lobbies(policy, ScriptedLobbies::default())
    }

    fn with_lobbies(policy: NetworkAgentPolicy, lobbies: ScriptedLobbies) -> Self {
        Self::build(policy, lobbies, Arc::new(UniformSourceWeights))
    }

    /// `source` 是一段共享出口，按来源计的名额乘上 `weight`。
    fn with_shared_source(source: [u8; 32], weight: u32) -> Self {
        Self::build(
            NetworkAgentPolicy::default_limits(true),
            ScriptedLobbies::default(),
            Arc::new(SharedSource { source, weight }),
        )
    }

    fn build(
        policy: NetworkAgentPolicy,
        lobbies: ScriptedLobbies,
        source_weights: Arc<dyn NetworkSourceWeights>,
    ) -> Self {
        let store = Arc::new(MemoryStore::default());
        let agents = Arc::new(FakeAgents::default());
        let lobbies = Arc::new(lobbies);
        let access = Arc::new(FakeAccess::default());
        let rooms = Arc::new(FakeRooms::default());
        let runtime = Arc::new(TestRuntime::new());
        let activity = Arc::new(RecordingActivity::default());
        let service = NetworkAgentService::new(NetworkAgentDependencies {
            store: store.clone(),
            sealer: Arc::new(MarkingSealer),
            keys: Arc::new(CountingKeys::default()),
            agents: agents.clone(),
            lobbies: lobbies.clone(),
            access: access.clone(),
            rooms: rooms.clone(),
            directory: Arc::new(FakeDirectory::standard()),
            secrets: Arc::new(CountingSecrets::default()),
            clock: runtime.clone(),
            identifiers: runtime.clone(),
            pause: runtime.clone(),
            policy,
            source_weights,
            activity: activity.clone(),
            matrix_server_name: "matrix.test".to_owned(),
        });
        Self {
            service,
            store,
            activity,
            agents,
            lobbies,
            access,
            rooms,
            runtime,
        }
    }

    fn enabled() -> Self {
        Self::new(NetworkAgentPolicy::default_limits(true))
    }

    fn generous() -> Self {
        Self::new(NetworkAgentPolicy {
            lobby_entries_per_source_per_hour: 100,
            lobby_entries_per_source_per_day: 100,
            ..NetworkAgentPolicy::default_limits(true)
        })
    }

    async fn create(
        &self,
        name: &str,
        room: Option<&str>,
    ) -> Result<CreatedNetworkAgent, NetworkAgentFailureKind> {
        self.create_from(name, room, [1; 32]).await
    }

    async fn create_from(
        &self,
        name: &str,
        room: Option<&str>,
        source_digest: [u8; 32],
    ) -> Result<CreatedNetworkAgent, NetworkAgentFailureKind> {
        self.service
            .create(CreateNetworkAgent {
                name: name.to_owned(),
                room: NetworkAgentRoomRequest::Lobby(room.map(str::to_owned)),
                source_digest,
            })
            .await
            .map_err(|failure| failure.kind())
    }

    async fn create_with_code(
        &self,
        name: &str,
        code: &str,
    ) -> Result<CreatedNetworkAgent, NetworkAgentFailureKind> {
        self.service
            .create(CreateNetworkAgent {
                name: name.to_owned(),
                room: NetworkAgentRoomRequest::Code(code.to_owned()),
                source_digest: [1; 32],
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

/// 进了哪一间：指名的就是那一间，否则是这个目录的大厅。
fn joined_in(
    catalog_id: RoomCatalogId,
    agent_instance_id: AgentInstanceId,
    room: Option<MatrixRoomReference>,
) -> EnterLobbyOutcome {
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
                matrix_room_id: room.unwrap_or_else(|| {
                    MatrixRoomReference::new(format!(
                        "!lobby-{}:matrix.test",
                        catalog_id.as_uuid().simple()
                    ))
                    .unwrap()
                }),
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

/// 新建时进了的或放行了的房间；只敲了门的没有。
fn room_of(created: &CreatedNetworkAgent) -> &NetworkAgentRoom {
    match &created.placement {
        NetworkAgentPlacement::Entered(room) | NetworkAgentPlacement::Admitted(room) => room,
        NetworkAgentPlacement::Knocked(knock) => panic!("只敲了门：{knock:?}"),
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

    assert_eq!(
        created.display_name,
        format!(
            "Scout {}",
            AgentNameTag::from_uuid(created.network_agent_id.as_uuid())
        ),
        "编号先按网络 Agent 的 ID 算"
    );
    assert_eq!(created.token.expose(), "token-1");
    assert_eq!(room_of(&created).catalog_id, catalog_id(2));
    assert_eq!(room_of(&created).name, "Agent Room 大厅");

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
        assert_eq!(stored.principal.display_name, created.display_name);
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
    // 名字已经带好编号，建宿主 Agent 时不再加。
    assert_eq!(host.display_name, created.display_name);
    assert_eq!(host.naming, AgentNaming::AsGiven);
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
    assert_eq!(me.display_name, created.display_name);
    assert_eq!(me.agent_id, created.agent_id);
    // 进过的大厅记在它名下，查看自己时带着房间名。
    assert_eq!(me.rooms, std::slice::from_ref(room_of(&created)));

    // 网关收发时取出的会话：Matrix 令牌是解封后的原文。
    let session = harness.service.session("token-1").await.expect("会话");
    assert_eq!(session.network_agent_id, created.network_agent_id);
    assert_eq!(session.agent_id, created.agent_id);
    assert_eq!(session.agent_instance_id, instance_id);
    assert_eq!(session.display_name, created.display_name);
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
    assert_eq!(
        session.rooms[0].matrix_room_id,
        room_of(&created).matrix_room_id
    );
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
async fn 一小时没动静先请出公开大厅_再调接口时回到原来的分片_不占名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        lobby_entries_per_source_per_hour: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let scout = harness.create("Scout", None).await.unwrap();
    let lobby = harness.store.rooms.lock().unwrap()[0].1.clone();
    harness.runtime.advance(HOUR);
    assert!(
        harness
            .service
            .idle_lobby_exits(10)
            .await
            .unwrap()
            .is_empty(),
        "刚好一小时还不算"
    );

    harness.runtime.advance(1);
    let exits = harness.service.idle_lobby_exits(10).await.unwrap();
    let [exit] = exits.as_slice() else {
        panic!("闲置的要请出大厅：{exits:?}");
    };
    assert_eq!(exit.id, scout.network_agent_id);
    assert_eq!(exit.room.matrix_room_id, lobby.matrix_room_id);
    assert!(exit.matrix_access_token.is_some(), "用它自己的会话离开");
    harness
        .service
        .mark_lobby_away(exit.id, &exit.room)
        .await
        .unwrap();
    assert!(
        harness
            .service
            .idle_lobby_exits(10)
            .await
            .unwrap()
            .is_empty()
    );

    let session = harness.service.session(scout.token.expose()).await.unwrap();
    assert_eq!(
        session
            .rooms
            .iter()
            .map(|room| room.matrix_room_id.clone())
            .collect::<Vec<_>>(),
        std::slice::from_ref(&lobby.matrix_room_id),
        "回到原来的分片"
    );
    assert!(session.lobbies_away.is_empty());
    let requests = harness.lobbies.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].target_room, Some(lobby.matrix_room_id));
    harness
        .create("Second", None)
        .await
        .expect("回大厅没占来源的名额，这一小时的那一个还在");
}

#[tokio::test]
async fn 原来的分片进不去时由大厅分配() {
    let harness = Harness::generous();
    let scout = harness.create("Scout", None).await.unwrap();
    harness.runtime.advance(HOUR + 1);
    let exit = harness
        .service
        .idle_lobby_exits(10)
        .await
        .unwrap()
        .remove(0);
    harness
        .service
        .mark_lobby_away(exit.id, &exit.room)
        .await
        .unwrap();
    harness
        .lobbies
        .script
        .lock()
        .unwrap()
        .push_back(Err(AgentLobbyEntryFailure::NotFound));

    let session = harness.service.session(scout.token.expose()).await.unwrap();
    assert_eq!(session.rooms.len(), 1);
    assert!(session.lobbies_away.is_empty());
    let requests = harness.lobbies.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.target_room.clone())
            .collect::<Vec<_>>(),
        [None, Some(exit.room.matrix_room_id), None]
    );
}

#[tokio::test]
async fn 大厅满了回不去时照常取会话_记着没回去的_空出位置后再回() {
    let harness = Harness::new(NetworkAgentPolicy {
        max_lobby_agents: 1,
        lobby_entries_per_source_per_hour: 2,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let scout = harness.create("Scout", None).await.unwrap();
    harness.runtime.advance(HOUR + 1);
    let exit = harness
        .service
        .idle_lobby_exits(10)
        .await
        .unwrap()
        .remove(0);
    harness
        .service
        .mark_lobby_away(exit.id, &exit.room)
        .await
        .unwrap();
    let other = harness
        .create("Other", None)
        .await
        .expect("请出去的不算在大厅的人数里");

    let session = harness.service.session(scout.token.expose()).await.unwrap();
    assert!(session.rooms.is_empty());
    assert_eq!(
        session
            .lobbies_away
            .iter()
            .map(|lobby| lobby.matrix_room_id.clone())
            .collect::<Vec<_>>(),
        std::slice::from_ref(&exit.room.matrix_room_id)
    );
    assert_eq!(
        harness
            .service
            .admit(
                scout.token.expose(),
                NetworkAgentRoomRequest::Lobby(None),
                [1; 32],
            )
            .await
            .err()
            .map(|failure| failure.kind()),
        Some(NetworkAgentFailureKind::CapacityReached),
        "大厅满了：只看满没满，不占名额"
    );

    harness
        .service
        .disable(other.token.expose())
        .await
        .expect("停用");
    let session = harness.service.session(scout.token.expose()).await.unwrap();
    assert_eq!(session.rooms.len(), 1, "空出位置后回来了");
    assert!(session.lobbies_away.is_empty());
}

#[tokio::test]
async fn 停用前离开房间用的会话不先回大厅() {
    let harness = Harness::generous();
    let scout = harness.create("Scout", None).await.unwrap();
    harness.runtime.advance(HOUR + 1);
    let exit = harness
        .service
        .idle_lobby_exits(10)
        .await
        .unwrap()
        .remove(0);
    harness
        .service
        .mark_lobby_away(exit.id, &exit.room)
        .await
        .unwrap();

    let session = harness
        .service
        .exit_session(scout.token.expose())
        .await
        .unwrap();
    assert!(session.rooms.is_empty());
    assert_eq!(harness.lobbies.requests.lock().unwrap().len(), 1);
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
    assert_tagged(&session.display_name, "Idle");
    assert_eq!(session.rooms.len(), 1, "带上它进过的大厅");
    harness
        .service
        .mark_rooms_left(idle.network_agent_id)
        .await
        .unwrap();
    assert!(harness.service.pending_exits(10).await.unwrap().is_empty());
    assert_keys_deleted_after_exit(&harness, idle.network_agent_id).await;
}

/// 离开房间以后等着删钥匙；删了以后封存的秘密一个不剩，也不再等着删。
async fn assert_keys_deleted_after_exit(harness: &Harness, id: NetworkAgentId) {
    assert_eq!(
        harness.service.pending_key_deletions(10).await.unwrap(),
        [id]
    );
    assert!(harness.store.with(id, |agent| !agent.secrets.is_empty()));
    harness.service.delete_keys(id).await.unwrap();
    assert!(harness.store.with(id, |agent| agent.secrets.is_empty()));
    assert!(
        harness
            .service
            .pending_key_deletions(10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn 加密存储口令与正文根密钥第一次要用时生成并封存_之后不变_解不开时不覆盖() {
    let harness = Harness::enabled();
    let created = harness.create("Cipher", None).await.unwrap();
    let id = created.network_agent_id;

    let first = harness.service.encryption_secrets(id).await.unwrap();
    assert_eq!(first.recovery_credential, None);
    assert_ne!(first.content_root_key, first.store_passphrase, "各是各的");
    let again = harness.service.encryption_secrets(id).await.unwrap();
    assert_eq!(again.store_passphrase, first.store_passphrase);
    assert_eq!(again.content_root_key, first.content_root_key);
    let root_key = harness.store.with(id, |agent| {
        agent
            .secrets
            .iter()
            .find(|(kind, _)| *kind == NetworkAgentSecretKind::MessageContentRootKey)
            .map(|(_, sealed)| sealed.clone())
            .unwrap()
    });
    assert_eq!(
        sealed_text(&root_key),
        format!(
            "sealed:{id}:message_content_root_key:{}",
            first.content_root_key.expose()
        )
    );
    let stored = harness.store.with(id, |agent| {
        agent
            .secrets
            .iter()
            .find(|(kind, _)| *kind == NetworkAgentSecretKind::MatrixStorePassphrase)
            .map(|(_, sealed)| sealed.clone())
            .unwrap()
    });
    assert_eq!(
        sealed_text(&stored),
        format!(
            "sealed:{id}:matrix_store_passphrase:{}",
            first.store_passphrase.expose()
        ),
        "交给存储的是封存后的值"
    );

    let credential = SecretValue::new("recovery-passphrase-for-backup").unwrap();
    harness
        .service
        .store_recovery_credential(id, &credential)
        .await
        .unwrap();
    assert_eq!(
        harness
            .service
            .encryption_secrets(id)
            .await
            .unwrap()
            .recovery_credential,
        Some(credential)
    );

    // 库里的口令被改过：解不开就报依赖不可用，也不生成新的去覆盖。
    {
        let mut agents = harness.store.agents.lock().unwrap();
        let stored = agents
            .iter_mut()
            .find(|agent| agent.record.id == id)
            .unwrap();
        for (kind, sealed) in &mut stored.secrets {
            if *kind == NetworkAgentSecretKind::MatrixStorePassphrase {
                sealed.bytes = b"tampered".to_vec();
            }
        }
    }
    assert_eq!(
        harness
            .service
            .encryption_secrets(id)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::DependencyUnavailable
    );
    let untouched = harness.store.with(id, |agent| {
        agent
            .secrets
            .iter()
            .filter(|(kind, _)| *kind == NetworkAgentSecretKind::MatrixStorePassphrase)
            .map(|(_, sealed)| sealed.bytes.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(untouched, [b"tampered".to_vec()]);
}

#[tokio::test]
async fn 会话带上实例的_matrix_设备与进加密房间的时刻() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let session = harness
        .service
        .session(created.token.expose())
        .await
        .unwrap();

    assert_eq!(
        session.matrix_device_id,
        format!("AR_{}", session.agent_instance_id.as_uuid().simple())
    );
    assert_eq!(session.encrypted_since, None);
}

#[tokio::test]
async fn 停用后会话打不开的_如实告诉清理方() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let principal_id = {
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
        stored.record.principal_id
    };
    harness
        .service
        .disable(created.token.expose())
        .await
        .unwrap();

    assert_eq!(
        harness.service.pending_exits(10).await.unwrap(),
        [NetworkAgentPendingExit::Unopenable {
            id: created.network_agent_id,
            principal_id,
        }]
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
    assert_eq!(room_of(&exact).catalog_id, catalog_id(1));
    let slug = harness.create("B", Some("RUST-NIGHT")).await.unwrap();
    assert_eq!(room_of(&slug).catalog_id, catalog_id(1));
    let lowered = harness.create("C", Some("general")).await.unwrap();
    assert_eq!(room_of(&lowered).catalog_id, catalog_id(3));

    let failure = harness
        .service
        .create(CreateNetworkAgent {
            name: "D".to_owned(),
            room: NetworkAgentRoomRequest::Lobby(Some("没有这间".to_owned())),
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
async fn 同名的都能进_名字后面加编号_撞上了换一个编号() {
    let harness = Harness::generous();

    let mut shown = HashSet::new();
    for name in ["Scout", "scout", "SCOUT #1234"] {
        let created = harness.create(name, None).await.unwrap();
        assert_tagged(&created.display_name, untagged_name(name));
        assert!(shown.insert(created.display_name.to_lowercase()));
    }
    // 带编号的名字被别人占了，换一个随机编号再试。
    *harness.store.names_taken.lock().unwrap() = 4;
    let retried = harness.create("Scout", None).await.unwrap();
    assert_tagged(&retried.display_name, "Scout");
    assert_ne!(
        retried.display_name,
        format!(
            "Scout {}",
            AgentNameTag::from_uuid(retried.network_agent_id.as_uuid())
        )
    );
    // 换了五次都撞上，才请 Agent 换个名字。
    *harness.store.names_taken.lock().unwrap() = 5;
    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::NameUnavailable)
    );
}

#[tokio::test]
async fn 大厅满了不扣来源的名额_空出位置后照样能进() {
    let harness = Harness::new(NetworkAgentPolicy {
        max_lobby_agents: 1,
        lobby_entries_per_source_per_hour: 2,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let first = harness.create("First", None).await.unwrap();
    for _ in 0..5 {
        assert_eq!(
            harness.create("Second", None).await.err(),
            Some(NetworkAgentFailureKind::CapacityReached)
        );
    }

    harness
        .service
        .disable(first.token.expose())
        .await
        .expect("停用成功");
    harness
        .create("Second", None)
        .await
        .expect("满着的时候没扣名额，空出位置后能进");
}

#[tokio::test]
async fn 没建成时退回起名的名额() {
    let harness = Harness::with_lobbies(
        NetworkAgentPolicy {
            lobby_entries_per_source_per_hour: 1,
            ..NetworkAgentPolicy::default_limits(true)
        },
        ScriptedLobbies::scripted(vec![Err(AgentLobbyEntryFailure::NotFound)]),
    );

    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::DependencyUnavailable)
    );
    harness
        .create("Scout", None)
        .await
        .expect("上一次没建成，名额退回来了");
    assert_eq!(
        harness.create("Scout", None).await.err(),
        Some(NetworkAgentFailureKind::RateLimited),
        "建成的那次照扣"
    );
}

#[tokio::test]
async fn 每小时和每天两档一起扣_有一档不够就都不扣() {
    let harness = Harness::new(NetworkAgentPolicy {
        lobby_entries_per_source_per_hour: 5,
        lobby_entries_per_source_per_day: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    harness.create("First", None).await.unwrap();
    assert_eq!(
        harness.create("Second", None).await.err(),
        Some(NetworkAgentFailureKind::RateLimited)
    );
    let hourly = harness
        .store
        .windows
        .lock()
        .unwrap()
        .iter()
        .find(|(bucket, _)| bucket.starts_with("create:hour:"))
        .map(|(_, (_, count))| *count);
    assert_eq!(hourly, Some(1), "每天的不够时每小时的也不扣");
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
            room: NetworkAgentRoomRequest::Lobby(None),
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
async fn 公开大厅里发言每分钟二十条_每天一千条_超了告诉何时能再发() {
    // 只看它自己的限额：整个大厅合起来的放到最大。
    let harness = Harness::new(NetworkAgentPolicy {
        lobby_total_messages_per_minute: 600,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let created = harness.create("Talker", None).await.unwrap();
    let id = created.network_agent_id;
    let lobby = room_of(&created).catalog_id;
    for _ in 0..20 {
        harness
            .service
            .take_message_quota(id, lobby)
            .await
            .expect("每分钟前二十条");
    }
    let limited = harness
        .service
        .take_message_quota(id, lobby)
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
                .take_message_quota(id, lobby)
                .await
                .unwrap_or_else(|_| panic!("第 {minute} 分钟"));
        }
    }
    harness.runtime.advance(60_000);
    let daily = harness
        .service
        .take_message_quota(id, lobby)
        .await
        .expect_err("一天一千条用完了");
    assert_eq!(daily.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        daily.retry_at(),
        Some(UtcMillis::new(START + 24 * HOUR).unwrap())
    );
}

#[tokio::test]
async fn 一个公开大厅的网络_agent_合起来每分钟十二条_自己超了的不占大厅的份额() {
    let harness = Harness::enabled();
    let first = harness.create("First", None).await.unwrap();
    let second = harness.create("Second", None).await.unwrap();
    let lobby = room_of(&first).catalog_id;
    assert_eq!(room_of(&second).catalog_id, lobby);
    for _ in 0..10 {
        harness
            .service
            .take_message_quota(first.network_agent_id, lobby)
            .await
            .expect("大厅的前十条");
    }
    for _ in 0..2 {
        harness
            .service
            .take_message_quota(second.network_agent_id, lobby)
            .await
            .expect("另一个 Agent 接着用大厅剩下的两条");
    }
    let busy = harness
        .service
        .take_message_quota(second.network_agent_id, lobby)
        .await
        .expect_err("大厅这一分钟的十二条用完了");
    assert_eq!(busy.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        busy.retry_at(),
        Some(UtcMillis::new(START + 60_000).unwrap())
    );
    let second_minute_used = harness
        .store
        .windows
        .lock()
        .unwrap()
        .get(&format!(
            "send:minute:{}",
            second.network_agent_id.as_uuid().simple()
        ))
        .map(|(_, count)| *count);
    assert_eq!(
        second_minute_used,
        Some(2),
        "大厅总额度不够时，不白扣它自己的发言额度"
    );

    // 下一分钟：先查它自己的，第一个说满二十条以后再说就被它自己的限额挡下，不再占大厅的。
    harness.runtime.advance(60_000);
    for _ in 0..12 {
        harness
            .service
            .take_message_quota(first.network_agent_id, lobby)
            .await
            .expect("新的一分钟，大厅又有十二条");
    }
    harness.runtime.advance(60_000);
    harness
        .service
        .take_message_quota(second.network_agent_id, lobby)
        .await
        .expect("第三分钟大厅又空出来了");
}

#[tokio::test]
async fn 总开关关着时不给发言额度() {
    let harness = Harness::new(NetworkAgentPolicy::default_limits(false));
    assert_eq!(
        harness
            .service
            .take_message_quota(NetworkAgentId::from_uuid(Uuid::now_v7()), catalog_id(2))
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Disabled
    );
}

#[tokio::test]
async fn 私人房间里说话不限条数_大厅里的额度用完了也照样说() {
    let harness = Harness::enabled();
    let created = harness.create_with_code("Cipher", CODE).await.unwrap();
    let id = created.network_agent_id;
    let private = room_of(&created).catalog_id;
    for _ in 0..1_100 {
        harness
            .service
            .take_message_quota(id, private)
            .await
            .expect("私人房间不限条数");
    }

    // 公开大厅里照样有限额：这里先到的是整个大厅每分钟十二条。
    for _ in 0..12 {
        harness
            .service
            .take_message_quota(id, catalog_id(2))
            .await
            .expect("同一个 Agent 在公开大厅里照样按限额算");
    }
    assert_eq!(
        harness
            .service
            .take_message_quota(id, catalog_id(2))
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::RateLimited
    );
    harness
        .service
        .take_message_quota(id, private)
        .await
        .expect("回到私人房间照样能说");
}

#[tokio::test]
async fn 共享出口整段算一个来源_进大厅和敲门的名额都乘上分量_别的来源照旧() {
    let shared = [7; 32];
    let harness = Harness::with_shared_source(shared, 3);
    for index in 0..15 {
        harness
            .create_from(&format!("Agent {index}"), None, shared)
            .await
            .expect("分量 3：一小时能进 15 个");
    }
    assert_eq!(
        harness.create_from("Agent 15", None, shared).await.err(),
        Some(NetworkAgentFailureKind::RateLimited)
    );
    for index in 0..5 {
        harness
            .create_from(&format!("Solo {index}"), None, [8; 32])
            .await
            .expect("普通来源照旧五个");
    }
    assert_eq!(
        harness.create_from("Solo 5", None, [8; 32]).await.err(),
        Some(NetworkAgentFailureKind::RateLimited)
    );

    let created = harness.create("Knocker", None).await.unwrap();
    for _ in 0..30 {
        harness
            .service
            .admit(created.token.expose(), knock_on_private(), shared)
            .await
            .expect("分量 3：一小时能敲 30 次");
    }
    let limited = harness
        .service
        .admit(created.token.expose(), knock_on_private(), shared)
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
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
            room: NetworkAgentRoomRequest::Lobby(None),
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
async fn 公开大厅里的网络_agent_到了全站上限就请稍后_停用一个就空出名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        max_lobby_agents: 1,
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
async fn 凭口令起名不占进大厅的名额_大厅满了照样进私人房间_只是进不了大厅() {
    let harness = Harness::new(NetworkAgentPolicy {
        max_lobby_agents: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    harness.create("Greeter", None).await.expect("大厅里第一个");
    assert_eq!(
        harness.create("Second", None).await.err(),
        Some(NetworkAgentFailureKind::CapacityReached)
    );

    // 同一个来源每小时只能让五个进大厅；凭口令进私人房间的不算，大厅满了也不挡。
    let mut ciphers = Vec::new();
    for index in 0..8 {
        ciphers.push(
            harness
                .create_with_code(&format!("Cipher {index}"), CODE)
                .await
                .unwrap_or_else(|failure| panic!("第 {index} 个：{failure:?}")),
        );
    }
    assert_eq!(
        harness
            .service
            .admit(
                ciphers[0].token.expose(),
                NetworkAgentRoomRequest::Lobby(None),
                [2; 32],
            )
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::CapacityReached,
        "进大厅照样看大厅满没满"
    );
}

#[tokio::test]
async fn 从私人房间头一回进公开大厅要占名额_已经在大厅里的换一间不算() {
    let harness = Harness::new(NetworkAgentPolicy {
        lobby_entries_per_source_per_hour: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let cipher = harness.create_with_code("Cipher", CODE).await.unwrap();
    let token = cipher.token.expose();
    harness
        .service
        .enter(token, NetworkAgentTarget::Private(room_of(&cipher).clone()))
        .await
        .expect("进私人房间");
    harness
        .create("Greeter", None)
        .await
        .expect("用掉这个来源这一小时的名额");

    let limited = harness
        .service
        .admit(token, NetworkAgentRoomRequest::Lobby(None), [1; 32])
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + HOUR).unwrap())
    );

    harness.runtime.advance(HOUR);
    let admitted = harness
        .service
        .admit(token, NetworkAgentRoomRequest::Lobby(None), [1; 32])
        .await
        .unwrap();
    let NetworkAgentAdmission::Admitted(target) = admitted else {
        panic!("过了一小时该放行了：{admitted:?}");
    };
    harness.service.enter(token, target).await.expect("进大厅");
    assert_eq!(
        harness
            .service
            .admit(
                token,
                NetworkAgentRoomRequest::Lobby(Some("rust-night".to_owned())),
                [1; 32],
            )
            .await
            .unwrap(),
        NetworkAgentAdmission::Admitted(NetworkAgentTarget::Lobby(catalog_id(1))),
        "已经在大厅里了，换一间不再占名额"
    );
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
    assert_tagged(&again.display_name, "Scout");
    assert_ne!(again.token.expose(), token);
}

#[tokio::test]
async fn 只有凭令牌认出来的网络_agent_才算在用() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let token = created.token.expose().to_owned();
    let agent = harness.store.only().id;
    assert!(harness.activity.seen().is_empty(), "建的时候还没凭令牌认过");

    for wrong in ["", "token-2"] {
        harness.service.me(wrong).await.unwrap_err();
    }
    assert!(harness.activity.seen().is_empty());

    harness.service.me(&token).await.unwrap();
    assert_eq!(harness.activity.seen(), [agent]);

    harness.service.disable(&token).await.expect("停用成功");
    harness.service.me(&token).await.unwrap_err();
    assert_eq!(harness.activity.seen(), [agent, agent], "停用以后不再算");
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
    assert_eq!(room_of(&created).catalog_id, catalog_id(3));
    assert_eq!(room_of(&created).name, "General");
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
    assert_tagged(&retried.display_name, "Scout");
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

#[tokio::test]
async fn 凭口令创建_先按来源核对口令_建好人物后以它的网络设备兑换_只放行不进房间() {
    let harness = Harness::enabled();

    let created = harness
        .create_with_code("Cipher", CODE)
        .await
        .expect("创建成功");

    assert!(
        matches!(created.placement, NetworkAgentPlacement::Admitted(_)),
        "私人房间要等网关准备好加密客户端再进"
    );
    assert_eq!(room_of(&created).catalog_id, catalog_id(9));
    assert_eq!(room_of(&created).matrix_room_id.as_str(), PRIVATE_ROOM);
    assert_eq!(room_of(&created).name, "项目室");
    assert!(harness.lobbies.catalogs().is_empty(), "不进大厅");
    assert_eq!(
        *harness.access.resolved.lock().unwrap(),
        [(JoinCodeCaller::NetworkSource([1; 32]), CODE.to_owned())]
    );
    let record = harness.store.only();
    assert_eq!(
        *harness.access.redeemed.lock().unwrap(),
        [(created.agent_id, record.device_id, CODE.to_owned())],
        "以这个网络 Agent 自己的网络设备兑换"
    );
    assert!(
        harness.store.rooms.lock().unwrap().is_empty(),
        "还没进，不记房间"
    );

    // 网关准备好之后进去：指名那一间，并记下来。
    let room = harness
        .service
        .enter(
            created.token.expose(),
            NetworkAgentTarget::Private(room_of(&created).clone()),
        )
        .await
        .expect("进私人房间");
    assert_eq!(&room, room_of(&created));
    let requests = harness.lobbies.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].catalog_id, catalog_id(9));
    assert_eq!(
        requests[0]
            .target_room
            .as_ref()
            .map(MatrixRoomReference::as_str),
        Some(PRIVATE_ROOM)
    );
    assert_eq!(requests[0].actor.device_id, record.device_id);
    let view = harness.service.me(created.token.expose()).await.unwrap();
    assert_eq!(view.rooms.len(), 1);
    assert_eq!(view.rooms[0].matrix_room_id.as_str(), PRIVATE_ROOM);
}

#[tokio::test]
async fn 口令不对时什么都不建_也不占创建名额_限流时告诉何时再试() {
    let harness = Harness::enabled();

    assert_eq!(
        harness
            .create_with_code("Guess", "0000-0000-0000")
            .await
            .unwrap_err(),
        NetworkAgentFailureKind::CodeInvalid
    );
    assert!(
        harness.store.agents.lock().unwrap().is_empty(),
        "什么都不建"
    );
    assert!(harness.access.redeemed.lock().unwrap().is_empty());
    for number in 0..5 {
        harness
            .create(&format!("Scout {number}"), None)
            .await
            .expect("猜错的那次不占这个来源每小时五个的名额");
    }

    let limited = harness
        .service
        .create(CreateNetworkAgent {
            name: "Again".to_owned(),
            room: NetworkAgentRoomRequest::Code("LIMITED".to_owned()),
            source_digest: [2; 32],
        })
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + HOUR).unwrap())
    );
}

#[tokio::test]
async fn 已有的网络_agent_再进公开大厅_已经在里面就原样返回_否则放行后由大厅分配() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let token = created.token.expose();

    let again = harness
        .service
        .admit(token, NetworkAgentRoomRequest::Lobby(None), [1; 32])
        .await
        .unwrap();
    assert_eq!(
        again,
        NetworkAgentAdmission::AlreadyIn(room_of(&created).clone())
    );

    let admitted = harness
        .service
        .admit(
            token,
            NetworkAgentRoomRequest::Lobby(Some("rust-night".to_owned())),
            [1; 32],
        )
        .await
        .unwrap();
    assert_eq!(
        admitted,
        NetworkAgentAdmission::Admitted(NetworkAgentTarget::Lobby(catalog_id(1)))
    );
    let NetworkAgentAdmission::Admitted(target) = admitted else {
        unreachable!()
    };
    let room = harness.service.enter(token, target).await.unwrap();
    assert_eq!(room.catalog_id, catalog_id(1));
    assert_eq!(room.name, "Rust 夜谈");
    let requests = harness.lobbies.requests.lock().unwrap().clone();
    assert_eq!(requests.last().unwrap().target_room, None, "大厅由分配决定");
    let view = harness.service.me(token).await.unwrap();
    assert_eq!(
        view.rooms
            .iter()
            .map(|room| room.catalog_id)
            .collect::<Vec<_>>(),
        [catalog_id(2), catalog_id(1)]
    );

    assert_eq!(
        harness
            .service
            .admit("wrong-token", NetworkAgentRoomRequest::Lobby(None), [1; 32])
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized
    );
}

#[tokio::test]
async fn 已有的网络_agent_凭口令进私人房间_被移出后拿旧口令只说口令无效() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let token = created.token.expose();

    let admitted = harness
        .service
        .admit(
            token,
            NetworkAgentRoomRequest::Code(CODE.to_owned()),
            [3; 32],
        )
        .await
        .unwrap();
    let NetworkAgentAdmission::Admitted(NetworkAgentTarget::Private(room)) = admitted else {
        panic!("凭口令放行的是私人房间：{admitted:?}");
    };
    assert_eq!(room.matrix_room_id.as_str(), PRIVATE_ROOM);
    assert_eq!(
        harness.access.resolved.lock().unwrap().last().unwrap().0,
        JoinCodeCaller::NetworkSource([3; 32]),
        "猜错计在来源上"
    );

    assert_eq!(
        harness
            .service
            .admit(
                token,
                NetworkAgentRoomRequest::Code("REMOVED".to_owned()),
                [3; 32]
            )
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::CodeInvalid
    );
    assert_eq!(
        harness
            .service
            .admit(
                token,
                NetworkAgentRoomRequest::Code("nope".to_owned()),
                [3; 32]
            )
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::CodeInvalid
    );
}

#[tokio::test]
async fn 记下第一次进加密房间的时刻_之后不改_会话随之带上() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let id = created.network_agent_id;

    let first = harness.service.mark_encrypted(id).await.unwrap();
    harness.runtime.advance(HOUR);
    assert_eq!(harness.service.mark_encrypted(id).await.unwrap(), first);
    let session = harness
        .service
        .session(created.token.expose())
        .await
        .unwrap();
    assert_eq!(session.encrypted_since, Some(first));
}

#[tokio::test]
async fn 加密存储重建时换一台设备_封存新的访问令牌_停用的不换() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let id = created.network_agent_id;
    let record = harness.store.only();

    let device = harness.service.replace_matrix_device(id).await.unwrap();
    let token = device.access_token;

    assert_eq!(device.device_id, "AR_NETWORK_1", "交回新设备");
    assert_eq!(token.expose(), format!("{MATRIX_TOKEN}-rotated-1"));
    let rotations = harness.agents.rotations.lock().unwrap().clone();
    assert_eq!(rotations.len(), 1);
    assert_eq!(rotations[0].instance_id, record.agent_instance_id.unwrap());
    assert_eq!(
        rotations[0].actor.device_id, record.device_id,
        "以它自己的网络设备换"
    );
    let session = harness
        .service
        .session(created.token.expose())
        .await
        .unwrap();
    assert_eq!(
        session.matrix_access_token, token,
        "之后的会话用新的访问令牌"
    );

    harness
        .service
        .disable(created.token.expose())
        .await
        .unwrap();
    assert_eq!(
        harness
            .service
            .replace_matrix_device(id)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert_eq!(harness.agents.rotations.lock().unwrap().len(), 1);
}

/// 敲门用的私人房间：只在 `FakeAccess` 里，不在公开大厅目录里。
fn private_catalog() -> RoomCatalogId {
    catalog_id(9)
}

fn knock_on_private() -> NetworkAgentRoomRequest {
    NetworkAgentRoomRequest::Lobby(Some(private_catalog().to_string()))
}

#[tokio::test]
async fn 拿房间号创建_先核对房间_建好人物后以它的网络设备敲门_不进任何房间() {
    let harness = Harness::enabled();
    let address = format!("https://agentroom.chat/lobby/{}", private_catalog());
    let created = harness.create("Scout", Some(&address)).await.unwrap();
    let NetworkAgentPlacement::Knocked(knock) = &created.placement else {
        panic!("拿私人房间的房间号是敲门：{:?}", created.placement);
    };
    assert_eq!(knock.catalog_id, private_catalog());
    assert_eq!(knock.status, NetworkAgentKnockStatus::Waiting);
    assert_eq!(knock.expires_at, UtcMillis::new(START + HOUR).unwrap());
    assert!(
        harness.lobbies.requests.lock().unwrap().is_empty(),
        "不进大厅"
    );
    assert!(
        harness.store.rooms.lock().unwrap().is_empty(),
        "不在任何房间里"
    );
    let record = harness.store.only();
    assert_eq!(record.status, NetworkAgentStatus::Active);
    assert_eq!(
        harness.access.knocked.lock().unwrap().clone(),
        [(created.agent_id, record.device_id, private_catalog())],
        "以它自己的网络设备敲门"
    );

    let view = harness.service.me(created.token.expose()).await.unwrap();
    assert!(view.rooms.is_empty());
    assert_eq!(view.knocks, std::slice::from_ref(knock));
    // 一直没人回答就作废。
    harness.runtime.advance(HOUR);
    let view = harness.service.me(created.token.expose()).await.unwrap();
    assert_eq!(view.knocks[0].status, NetworkAgentKnockStatus::Expired);
}

#[tokio::test]
async fn 房间号不对时什么都不建_是公开大厅的就直接进() {
    let harness = Harness::enabled();
    let unknown = catalog_id(404).to_string();
    assert_eq!(
        harness.create("Scout", Some(&unknown)).await.err(),
        Some(NetworkAgentFailureKind::RoomNotFound)
    );
    assert!(
        harness.store.agents.lock().unwrap().is_empty(),
        "房间号不对什么都不建"
    );
    assert!(harness.access.knocked.lock().unwrap().is_empty());

    let lobby = harness
        .create("Scout", Some(&catalog_id(1).to_string()))
        .await
        .unwrap();
    assert!(matches!(lobby.placement, NetworkAgentPlacement::Entered(_)));
    assert_eq!(room_of(&lobby).catalog_id, catalog_id(1));
    assert!(harness.access.knocked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 已有的网络_agent_拿房间号敲门_成员直接放行_没让进的照实说_在里面的原样返回() {
    let harness = Harness::enabled();
    let created = harness.create("Scout", None).await.unwrap();
    let token = created.token.expose();

    let knocked = harness
        .service
        .admit(token, knock_on_private(), [3; 32])
        .await
        .unwrap();
    let NetworkAgentAdmission::Knocked(knock) = knocked else {
        panic!("敲门：{knocked:?}");
    };
    assert_eq!(knock.status, NetworkAgentKnockStatus::Waiting);

    harness
        .access
        .declined
        .lock()
        .unwrap()
        .insert(created.agent_id);
    let declined = harness
        .service
        .admit(token, knock_on_private(), [3; 32])
        .await
        .unwrap();
    assert!(
        matches!(&declined, NetworkAgentAdmission::Knocked(knock) if knock.status == NetworkAgentKnockStatus::Declined),
        "{declined:?}"
    );
    harness.access.declined.lock().unwrap().clear();

    harness
        .access
        .members
        .lock()
        .unwrap()
        .insert(created.agent_id);
    let admitted = harness
        .service
        .admit(token, knock_on_private(), [3; 32])
        .await
        .unwrap();
    let NetworkAgentAdmission::Admitted(target) = admitted else {
        panic!("已经是 Agent 成员的直接放行：{admitted:?}");
    };
    let room = harness.service.enter(token, target).await.unwrap();
    assert_eq!(room.matrix_room_id.as_str(), PRIVATE_ROOM);
    assert_eq!(
        harness
            .service
            .admit(token, knock_on_private(), [3; 32])
            .await
            .unwrap(),
        NetworkAgentAdmission::AlreadyIn(NetworkAgentRoom {
            name: String::new(),
            ..room
        }),
        "已经在里面：原样返回"
    );
    let view = harness.service.me(token).await.unwrap();
    assert!(view.knocks.is_empty(), "进来以后不再列这扇门");
}

#[tokio::test]
async fn 敲门每个来源每小时十次_拿房间号建人物也算_不占进大厅的名额() {
    let harness = Harness::enabled();
    let address = private_catalog().to_string();
    for index in 0..10 {
        harness
            .create_from(&format!("Scout {index}"), Some(&address), [7; 32])
            .await
            .unwrap();
    }
    assert_eq!(
        harness
            .create_from("Scout 10", Some(&address), [7; 32])
            .await
            .err(),
        Some(NetworkAgentFailureKind::RateLimited)
    );
    let created = harness.create_from("Other", None, [8; 32]).await.unwrap();
    for _ in 0..10 {
        harness
            .service
            .admit(created.token.expose(), knock_on_private(), [8; 32])
            .await
            .unwrap();
    }
    let limited = harness
        .service
        .admit(created.token.expose(), knock_on_private(), [8; 32])
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert!(limited.retry_at().is_some());
    harness
        .service
        .admit(created.token.expose(), knock_on_private(), [9; 32])
        .await
        .expect("别的来源不受影响");
}

#[tokio::test]
async fn 放行后按_agent_取会话替它进房间_不记活动_停用的不行() {
    let harness = Harness::enabled();
    let created = harness
        .create("Scout", Some(&private_catalog().to_string()))
        .await
        .unwrap();
    harness.runtime.advance(60_000);
    let session = harness
        .service
        .session_of_agent(created.agent_id)
        .await
        .unwrap();
    assert_eq!(session.agent_id, created.agent_id);
    assert!(session.rooms.is_empty());
    assert_eq!(
        harness.store.only().last_active_at,
        UtcMillis::new(START).unwrap(),
        "替它进房间不算它的活动"
    );

    let room = harness
        .service
        .enter_admitted(
            created.agent_id,
            NetworkAgentRoom {
                catalog_id: private_catalog(),
                matrix_room_id: MatrixRoomReference::new(PRIVATE_ROOM.to_owned()).unwrap(),
                name: "项目室".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(room.matrix_room_id.as_str(), PRIVATE_ROOM);
    assert_eq!(room.name, "项目室");
    let request = harness
        .lobbies
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(
        request.target_room.map(|room| room.as_str().to_owned()),
        Some(PRIVATE_ROOM.to_owned()),
        "指名进那个私人房间"
    );
    let session = harness
        .service
        .session_of_agent(created.agent_id)
        .await
        .unwrap();
    assert_eq!(session.rooms.len(), 1);

    let stranger = AgentId::from_uuid(Uuid::from_u128(0x0198_b601_0000_7000_8000_0000_00ff_0000));
    assert_eq!(
        harness
            .service
            .session_of_agent(stranger)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized,
        "不是网络 Agent"
    );
    harness
        .service
        .disable(created.token.expose())
        .await
        .unwrap();
    assert_eq!(
        harness
            .service
            .session_of_agent(created.agent_id)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert_eq!(
        harness
            .service
            .enter_admitted(created.agent_id, room)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::Unauthorized
    );
}

#[tokio::test]
async fn 放行后进大厅失败时退回名额_重进房间后迟到的请出不误删新记录() {
    let harness = Harness::new(NetworkAgentPolicy {
        lobby_entries_per_source_per_hour: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let scout = harness
        .create("Scout", Some(&private_catalog().to_string()))
        .await
        .unwrap();

    let admitted = harness
        .service
        .admit(
            scout.token.expose(),
            NetworkAgentRoomRequest::Lobby(None),
            [5; 32],
        )
        .await
        .expect("第一次放行进大厅");
    let NetworkAgentAdmission::Admitted(target) = admitted else {
        panic!("公开大厅应当直接放行：{admitted:?}");
    };
    harness
        .lobbies
        .script
        .lock()
        .unwrap()
        .push_back(Err(AgentLobbyEntryFailure::NotFound));
    assert_eq!(
        harness
            .service
            .enter(scout.token.expose(), target)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::DependencyUnavailable
    );
    let retried = harness
        .service
        .admit(
            scout.token.expose(),
            NetworkAgentRoomRequest::Lobby(None),
            [5; 32],
        )
        .await
        .expect("进房失败退回名额后可再次放行");
    let NetworkAgentAdmission::Admitted(target) = retried else {
        panic!("公开大厅应当直接放行：{retried:?}");
    };
    let entered = harness
        .service
        .enter(scout.token.expose(), target)
        .await
        .expect("第二次进第一个大厅成功");

    // 中间被请出又进来过一次：房间记录是后来那次进的时间。重复记同一个房间不改最初的时间，
    // 所以直接改记录。
    harness.runtime.advance(5_000);
    for (agent, room) in harness.store.rooms.lock().unwrap().iter_mut() {
        if *agent == scout.network_agent_id && room.matrix_room_id == entered.matrix_room_id {
            room.joined_at = UtcMillis::new(START + 5_000).unwrap();
        }
    }
    let older_exit = NetworkAgentRoomRecord {
        catalog_id: entered.catalog_id,
        matrix_room_id: entered.matrix_room_id,
        joined_at: UtcMillis::new(START).unwrap(),
        name: None,
    };
    harness
        .service
        .mark_lobby_away(scout.network_agent_id, &older_exit)
        .await
        .unwrap();
    assert!(
        harness.store.away.lock().unwrap().is_empty(),
        "迟到的旧请出记录不覆盖已经重新进房间的状态"
    );
    assert!(
        harness
            .store
            .rooms
            .lock()
            .unwrap()
            .iter()
            .any(|(agent, _)| *agent == scout.network_agent_id),
        "后来那次进房间的记录还在"
    );
}

#[tokio::test]
async fn 已在别的大厅时即使在场人数已满也能回到暂离的其余大厅() {
    let harness = Harness::new(NetworkAgentPolicy {
        max_lobby_agents: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let scout = harness.create("Scout", None).await.unwrap();
    let second_admitted = harness
        .service
        .admit(
            scout.token.expose(),
            NetworkAgentRoomRequest::Lobby(Some("rust-night".to_owned())),
            [6; 32],
        )
        .await
        .expect("放行进第二个大厅");
    let NetworkAgentAdmission::Admitted(second_target) = second_admitted else {
        panic!("第二个大厅应当直接放行：{second_admitted:?}");
    };
    let second_entered = harness
        .service
        .enter(scout.token.expose(), second_target)
        .await
        .expect("进第二个大厅成功");
    let away_record = NetworkAgentRoomRecord {
        catalog_id: second_entered.catalog_id,
        matrix_room_id: second_entered.matrix_room_id,
        joined_at: UtcMillis::new(START).unwrap(),
        name: Some(second_entered.name),
    };
    harness
        .service
        .mark_lobby_away(scout.network_agent_id, &away_record)
        .await
        .unwrap();
    let returned = harness.service.session(scout.token.expose()).await.unwrap();
    assert_eq!(
        returned.rooms.len(),
        2,
        "它自己已经在一个大厅里时，能回到暂离的另一个大厅"
    );
}

#[path = "network_agent_flow/rooms.rs"]
mod rooms;
