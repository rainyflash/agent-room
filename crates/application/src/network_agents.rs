//! 只凭网络接入的 Agent（ADR 0010、`specs/network-agents/design.md`）。
//!
//! 服务器替 Agent 保管身份：一个合成主体、一台已验证的网络设备、归这个主体所有的 Agent 与
//! 实例、封存的签名种子和 Matrix 会话。Agent 自己只拿一个访问令牌。创建时复用本机 Bridge
//! 走的同一套用例（建宿主 Agent、登记实例、进大厅），只是由服务器代表这台设备。

use std::sync::Arc;

use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    identity::Principal,
    ids::{
        AgentCreationRequestId, AgentId, AgentInstanceRegistrationRequestId, NetworkAgentId,
        RoomCatalogId,
    },
    network_agents::{NetworkAgentName, NetworkAgentStatus},
    rooms::{MatrixRoomReference, RoomSlug},
    time::{DurationMillis, UtcMillis},
};
use serde_json::Map;

use crate::{
    agent_lobbies::{AgentLobbyEntryUseCases, EnterAgentLobby},
    agents::{AgentManagementUseCases, CreateHostAgentForDevice, RegisterAgentInstance},
    devices::AuthenticatedDevice,
    persistence::RepositoryError,
    ports::{
        Clock, IdentifierFactory, NetworkAgentActivation, NetworkAgentBeginOutcome,
        NetworkAgentKeyFactory, NetworkAgentPause, NetworkAgentProvisioning, NetworkAgentRecord,
        NetworkAgentSecretKind, NetworkAgentSecretSealer, NetworkAgentStore, PortFuture,
        PrincipalAccount, RateWindowDecision, RateWindowPolicy, RoomDirectory, RoomDirectoryQuery,
        SecretFactory, SecretValue,
    },
    rooms::EnterLobbyOutcome,
};

/// 默认公开大厅的 slug（迁移 `202608280001_default_public_lobby.sql`）。
const DEFAULT_LOBBY_SLUG: &str = "agent-room-global";
const DEVICE_LABEL: &str = "Agent Room 网络 Agent";
const ADAPTER_TYPE: &str = "network";
const CAPABILITY_VERSION: &str = "1.0";
/// 合成主体的会话只活在这一次请求里：够建 Agent、登记实例、进大厅。
const ACTOR_LIFETIME_MILLIS: u64 = 5 * 60 * 1_000;
/// 同名时最多试到 ` 20`，再撞上就请 Agent 换个名字。
const MAX_NAME_NUMBER: u32 = 20;
/// 大厅正在准备房间时最多等这么多次。
const MAX_LOBBY_ATTEMPTS: usize = 3;

/// 部署配置里的开关与限额。总开关默认关闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkAgentPolicy {
    pub enabled: bool,
    pub creations_per_source_per_hour: u32,
    pub creations_per_source_per_day: u32,
    pub max_live_agents: u64,
}

impl NetworkAgentPolicy {
    /// 设计文档里的初始值：每个来源每小时 5 个、每天 20 个；全站同时 500 个。
    pub const fn default_limits(enabled: bool) -> Self {
        Self {
            enabled,
            creations_per_source_per_hour: 5,
            creations_per_source_per_day: 20,
            max_live_agents: 500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateNetworkAgent {
    pub name: String,
    /// 公开大厅的名字或 slug；省略就进默认公开大厅。
    pub room: Option<String>,
    /// 来源地址按天加盐后的摘要，只用于限流。
    pub source_digest: [u8; 32],
}

/// 网络 Agent 所在的公开大厅。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentRoom {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedNetworkAgent {
    pub network_agent_id: NetworkAgentId,
    pub agent_id: AgentId,
    /// 实际用的名字：同名时带序号。
    pub display_name: String,
    /// 只在这一次返回；库里只存摘要。
    pub token: SecretValue,
    pub room: NetworkAgentRoom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentView {
    pub network_agent_id: NetworkAgentId,
    pub agent_id: AgentId,
    pub display_name: String,
    pub created_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentFailureKind {
    /// 总开关关着。
    Disabled,
    InvalidName,
    /// 同名的太多，请换个名字。
    NameUnavailable,
    RoomNotFound,
    RateLimited,
    /// 全站同时有效的网络 Agent 到了上限。
    CapacityReached,
    /// 令牌不对，或这个网络 Agent 已停用。
    Unauthorized,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentFailure {
    kind: NetworkAgentFailureKind,
    retry_at: Option<UtcMillis>,
    /// 找不到房间时列出能进的公开大厅。
    rooms: Vec<String>,
}

impl NetworkAgentFailure {
    pub const fn new(kind: NetworkAgentFailureKind) -> Self {
        Self {
            kind,
            retry_at: None,
            rooms: Vec::new(),
        }
    }

    pub const fn rate_limited(retry_at: UtcMillis) -> Self {
        Self {
            kind: NetworkAgentFailureKind::RateLimited,
            retry_at: Some(retry_at),
            rooms: Vec::new(),
        }
    }

    /// 带上能进的公开大厅，方便 Agent 换个名字重试。
    pub const fn room_not_found(rooms: Vec<String>) -> Self {
        Self {
            kind: NetworkAgentFailureKind::RoomNotFound,
            retry_at: None,
            rooms,
        }
    }

    pub const fn kind(&self) -> NetworkAgentFailureKind {
        self.kind
    }

    pub const fn retry_at(&self) -> Option<UtcMillis> {
        self.retry_at
    }

    pub fn rooms(&self) -> &[String] {
        &self.rooms
    }
}

pub type NetworkAgentResult<T> = Result<T, NetworkAgentFailure>;

pub trait NetworkAgentUseCases: Send + Sync {
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>>;

    fn me<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>>;

    /// 停用：令牌立即作废，之后再用它只会得到“未认证”。
    fn disable<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>>;
}

pub struct NetworkAgentDependencies {
    pub store: Arc<dyn NetworkAgentStore>,
    pub sealer: Arc<dyn NetworkAgentSecretSealer>,
    pub keys: Arc<dyn NetworkAgentKeyFactory>,
    pub agents: Arc<dyn AgentManagementUseCases>,
    pub lobbies: Arc<dyn AgentLobbyEntryUseCases>,
    pub directory: Arc<dyn RoomDirectory>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub pause: Arc<dyn NetworkAgentPause>,
    pub policy: NetworkAgentPolicy,
    /// Matrix 服务器名，用来算合成主体的 Matrix 用户 ID（这个用户从不注册）。
    pub matrix_server_name: String,
}

pub struct NetworkAgentService {
    store: Arc<dyn NetworkAgentStore>,
    sealer: Arc<dyn NetworkAgentSecretSealer>,
    keys: Arc<dyn NetworkAgentKeyFactory>,
    agents: Arc<dyn AgentManagementUseCases>,
    lobbies: Arc<dyn AgentLobbyEntryUseCases>,
    directory: Arc<dyn RoomDirectory>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
    identifiers: Arc<dyn IdentifierFactory>,
    pause: Arc<dyn NetworkAgentPause>,
    policy: NetworkAgentPolicy,
    matrix_server_name: String,
}

impl NetworkAgentService {
    pub fn new(dependencies: NetworkAgentDependencies) -> Self {
        Self {
            store: dependencies.store,
            sealer: dependencies.sealer,
            keys: dependencies.keys,
            agents: dependencies.agents,
            lobbies: dependencies.lobbies,
            directory: dependencies.directory,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
            identifiers: dependencies.identifiers,
            pause: dependencies.pause,
            policy: dependencies.policy,
            matrix_server_name: dependencies.matrix_server_name,
        }
    }

    async fn create_internal(
        &self,
        request: CreateNetworkAgent,
    ) -> NetworkAgentResult<CreatedNetworkAgent> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        let name = NetworkAgentName::parse(&request.name)
            .map_err(|_| NetworkAgentFailure::new(NetworkAgentFailureKind::InvalidName))?;
        let catalog = self.lobby(request.room.as_deref()).await?;
        self.limit(&request.source_digest).await?;
        if self.store.count_live().await.map_err(repository)? >= self.policy.max_live_agents {
            return Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::CapacityReached,
            ));
        }

        let now = self.clock.now();
        let id = NetworkAgentId::from_uuid(uuid::Uuid::now_v7());
        let token = self.secrets.generate().map_err(|_| internal())?;
        let device_key = self.keys.generate().map_err(|_| internal())?;
        let instance_key = self.keys.generate().map_err(|_| internal())?;
        let principal_id = self.identifiers.principal_id();
        let device = Device::register(
            self.identifiers.device_id(),
            principal_id,
            DEVICE_LABEL.to_owned(),
            DevicePlatform::Network,
            DevicePublicSigningKey::new(device_key.public_key.to_vec()).map_err(|_| internal())?,
            now,
        )
        .and_then(|mut device| device.verify().map(|()| device))
        .map_err(|_| internal())?;
        let secrets = vec![
            (
                NetworkAgentSecretKind::DeviceSigningSeed,
                self.seal(id, NetworkAgentSecretKind::DeviceSigningSeed, &device_key)?,
            ),
            (
                NetworkAgentSecretKind::InstanceSigningSeed,
                self.seal(
                    id,
                    NetworkAgentSecretKind::InstanceSigningSeed,
                    &instance_key,
                )?,
            ),
        ];

        // 名字先占住：没停用的网络 Agent 不重名（不分大小写），撞上了就加序号再试。
        let mut provisioning = NetworkAgentProvisioning {
            id,
            principal: crate::principal_projection::network_agent_registration(
                principal_id,
                &id.to_string(),
                name.as_str(),
                now,
                &self.matrix_server_name,
            ),
            device,
            device_label: DEVICE_LABEL.to_owned(),
            token_digest: self.secrets.digest(token.expose()),
            display_name: name.as_str().to_owned(),
            source_digest: request.source_digest,
            secrets,
            created_at: now,
        };
        let mut number = 1;
        loop {
            name.numbered(number)
                .as_str()
                .clone_into(&mut provisioning.display_name);
            provisioning
                .principal
                .display_name
                .clone_from(&provisioning.display_name);
            match self.store.begin(&provisioning).await.map_err(repository)? {
                NetworkAgentBeginOutcome::Created => break,
                NetworkAgentBeginOutcome::NameTaken if number < MAX_NAME_NUMBER => number += 1,
                NetworkAgentBeginOutcome::NameTaken => {
                    return Err(NetworkAgentFailure::new(
                        NetworkAgentFailureKind::NameUnavailable,
                    ));
                }
            }
        }

        // 从这里起失败时令牌不会交给 Agent，这条记录也就没用了：停用它，放开名字与全站名额。
        // 停用本身失败时记录留在 provisioning，仍占着名字，但不影响这次的结果。
        match self
            .activate_and_enter(&provisioning, instance_key.public_key, catalog, now)
            .await
        {
            Ok((agent_id, room)) => Ok(CreatedNetworkAgent {
                network_agent_id: id,
                agent_id,
                display_name: provisioning.display_name,
                token,
                room,
            }),
            Err(failure) => {
                let _released = self.store.disable(id, self.clock.now()).await;
                Err(failure)
            }
        }
    }

    /// 以这台网络设备的身份建 Agent、登记实例、保存 Matrix 会话，生效后进大厅。
    async fn activate_and_enter(
        &self,
        provisioning: &NetworkAgentProvisioning,
        instance_public_key: [u8; 32],
        catalog: RoomCatalogId,
        now: UtcMillis,
    ) -> NetworkAgentResult<(AgentId, NetworkAgentRoom)> {
        let id = provisioning.id;
        let actor = AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(provisioning.principal.principal.id()),
                matrix_user_id: provisioning.principal.matrix_user_id.clone(),
                display_name: provisioning.display_name.clone(),
                avatar_content_id: None,
                locale: provisioning.principal.locale.clone(),
            },
            device_id: provisioning.device.id(),
            access_token_expires_at: now
                .checked_add(DurationMillis::new(ACTOR_LIFETIME_MILLIS).map_err(|_| internal())?)
                .map_err(|_| internal())?,
        };
        // 两步都按网络 Agent 的 ID 幂等：重试回到同一个 Agent 与实例。
        let agent = self
            .agents
            .create_host_agent_for_device(CreateHostAgentForDevice {
                request_id: AgentCreationRequestId::from_uuid(id.as_uuid()),
                actor: actor.clone(),
                display_name: provisioning.display_name.clone(),
            })
            .await
            .map_err(|_| dependency())?;
        let instance = self
            .agents
            .register_instance(RegisterAgentInstance {
                request_id: AgentInstanceRegistrationRequestId::from_uuid(id.as_uuid()),
                actor: actor.clone(),
                agent_id: agent.agent.id(),
                adapter_type: ADAPTER_TYPE.to_owned(),
                external_subject_hash: None,
                capability_version: CAPABILITY_VERSION.to_owned(),
                configuration: Map::new(),
                public_signing_key: AgentInstancePublicSigningKey::new(
                    instance_public_key.to_vec(),
                )
                .map_err(|_| internal())?,
            })
            .await
            .map_err(|_| dependency())?;
        let agent_instance_id = instance.registration.instance.id();
        let matrix_access_token = self
            .sealer
            .seal(
                id,
                NetworkAgentSecretKind::MatrixAccessToken,
                instance.matrix_session.access_token().expose().as_bytes(),
            )
            .map_err(|_| dependency())?;
        self.store
            .activate(&NetworkAgentActivation {
                id,
                agent_id: agent.agent.id(),
                agent_instance_id,
                matrix_access_token,
                activated_at: self.clock.now(),
            })
            .await
            .map_err(repository)?;

        let room = self
            .enter(&actor, agent.agent.id(), agent_instance_id, catalog)
            .await?;
        Ok((agent.agent.id(), room))
    }

    async fn me_internal(&self, token: &str) -> NetworkAgentResult<NetworkAgentView> {
        let record = self.authenticated(token).await?;
        let agent_id = record.agent_id.ok_or_else(internal)?;
        self.store
            .record_activity(record.id, self.clock.now())
            .await
            .map_err(repository)?;
        Ok(NetworkAgentView {
            network_agent_id: record.id,
            agent_id,
            display_name: record.display_name,
            created_at: record.created_at,
        })
    }

    async fn disable_internal(&self, token: &str) -> NetworkAgentResult<()> {
        let record = self.authenticated(token).await?;
        self.store
            .disable(record.id, self.clock.now())
            .await
            .map_err(repository)
    }

    /// 只认生效中的网络 Agent；总开关关着时一律拒绝，哪怕没带令牌。
    async fn authenticated(&self, token: &str) -> NetworkAgentResult<NetworkAgentRecord> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        if token.is_empty() {
            return Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            ));
        }
        let digest = self.secrets.digest(token);
        self.store
            .find_by_token(&digest)
            .await
            .map_err(repository)?
            .filter(|record| record.status == NetworkAgentStatus::Active)
            .ok_or_else(|| NetworkAgentFailure::new(NetworkAgentFailureKind::Unauthorized))
    }

    /// 按名字或 slug 找公开大厅：先精确，再忽略大小写；不给名字就是默认公开大厅。
    async fn lobby(&self, wanted: Option<&str>) -> NetworkAgentResult<RoomCatalogId> {
        let lobbies = self
            .directory
            .list_public(&RoomDirectoryQuery::default())
            .await
            .map_err(repository)?;
        let wanted = wanted.map(str::trim).filter(|name| !name.is_empty());
        let found = match wanted {
            None => lobbies
                .iter()
                .find(|entry| {
                    entry.catalog.slug().map(RoomSlug::as_str) == Some(DEFAULT_LOBBY_SLUG)
                })
                .or_else(|| lobbies.first()),
            Some(name) => lobbies
                .iter()
                .find(|entry| {
                    entry.catalog.name() == name
                        || entry.catalog.slug().map(RoomSlug::as_str) == Some(name)
                })
                .or_else(|| {
                    let lowered = name.to_lowercase();
                    let mut matches = lobbies.iter().filter(|entry| {
                        entry.catalog.name().to_lowercase() == lowered
                            || entry
                                .catalog
                                .slug()
                                .is_some_and(|slug| slug.as_str().to_lowercase() == lowered)
                    });
                    let first = matches.next();
                    // 忽略大小写后仍有多间同名时不猜，当作找不到并列出候选。
                    first.filter(|_| matches.next().is_none())
                }),
        };
        found.map(|entry| entry.catalog.id()).ok_or_else(|| {
            NetworkAgentFailure::room_not_found(
                lobbies
                    .iter()
                    .map(|entry| entry.catalog.name().to_owned())
                    .collect(),
            )
        })
    }

    /// 同一来源每小时、每天各有上限；超过时告诉 Agent 什么时候能再试。
    async fn limit(&self, source: &[u8; 32]) -> NetworkAgentResult<()> {
        let source = hex(source);
        for (window, limit, label) in [
            (
                3_600_000_u64,
                self.policy.creations_per_source_per_hour,
                "hour",
            ),
            (
                86_400_000_u64,
                self.policy.creations_per_source_per_day,
                "day",
            ),
        ] {
            let bucket = format!("create:{label}:{source}");
            let policy = RateWindowPolicy {
                window: DurationMillis::new(window).map_err(|_| internal())?,
                limit,
            };
            if let RateWindowDecision::Limited { retry_at } = self
                .store
                .take(&bucket, self.clock.now(), policy)
                .await
                .map_err(repository)?
            {
                return Err(NetworkAgentFailure::rate_limited(retry_at));
            }
        }
        Ok(())
    }

    /// 进公开大厅。大厅正在准备新房间时按服务器给的时间等一会儿再试。
    async fn enter(
        &self,
        actor: &AuthenticatedDevice,
        agent_id: AgentId,
        agent_instance_id: agent_room_domain::ids::AgentInstanceId,
        catalog_id: RoomCatalogId,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        let mut catalog_id = catalog_id;
        for _ in 0..MAX_LOBBY_ATTEMPTS {
            let outcome = self
                .lobbies
                .enter(EnterAgentLobby {
                    actor: actor.clone(),
                    agent_id,
                    agent_instance_id,
                    catalog_id,
                    preferred_language: None,
                    preferred_region: None,
                    target_room: None,
                })
                .await
                .map_err(|_| dependency())?;
            match outcome {
                EnterLobbyOutcome::Joined { room, .. } => {
                    let name = self
                        .directory
                        .find_catalog(room.catalog_id())
                        .await
                        .map_err(repository)?
                        .map_or_else(String::new, |catalog| catalog.name().to_owned());
                    return Ok(NetworkAgentRoom {
                        catalog_id: room.catalog_id(),
                        matrix_room_id: room.matrix_room_id().clone(),
                        name,
                    });
                }
                EnterLobbyOutcome::ProvisioningBusy { retry_at } => {
                    self.pause.until(retry_at).await;
                }
                EnterLobbyOutcome::CapacityChanged {
                    catalog_id: changed,
                } => {
                    catalog_id = changed;
                }
            }
        }
        Err(dependency())
    }

    fn seal(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        key: &crate::ports::GeneratedSigningKey,
    ) -> NetworkAgentResult<crate::ports::SealedSecret> {
        self.sealer
            .seal(id, kind, key.encoded_seed.expose().as_bytes())
            .map_err(|_| dependency())
    }
}

impl NetworkAgentUseCases for NetworkAgentService {
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
        Box::pin(self.create_internal(request))
    }

    fn me<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>> {
        Box::pin(self.me_internal(token))
    }

    fn disable<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.disable_internal(token))
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

fn repository(_error: RepositoryError) -> NetworkAgentFailure {
    dependency()
}

const fn dependency() -> NetworkAgentFailure {
    NetworkAgentFailure::new(NetworkAgentFailureKind::DependencyUnavailable)
}

const fn internal() -> NetworkAgentFailure {
    NetworkAgentFailure::new(NetworkAgentFailureKind::Internal)
}
