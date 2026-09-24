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
        AgentCreationRequestId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId,
        NetworkAgentId, PrincipalId, RoomCatalogId,
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
        Clock, IdentifierFactory, MatrixAgentLocalpart, NetworkAgentActivation,
        NetworkAgentBeginOutcome, NetworkAgentKeyFactory, NetworkAgentPause,
        NetworkAgentProvisioning, NetworkAgentRecord, NetworkAgentRoomRecord,
        NetworkAgentSecretKind, NetworkAgentSecretSealer, NetworkAgentStaleCutoff,
        NetworkAgentStore, PortFuture, PrincipalAccount, RateWindowDecision, RateWindowPolicy,
        RoomDirectory, RoomDirectoryQuery, SecretFactory, SecretValue,
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
/// 30 天没有活动的网络 Agent 自动停用，放开全站名额。
const IDLE_LIFETIME_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;
/// 创建一个小时还没建好，说明中途断了：停用，放开名字与名额。
const PROVISIONING_TIMEOUT_MILLIS: i64 = 60 * 60 * 1_000;
/// 定时清理每轮最多停用这么多个。
const STALE_BATCH: u32 = 100;
/// 大厅正在准备房间时最多等这么多次。
const MAX_LOBBY_ATTEMPTS: usize = 3;

/// 部署配置里的开关与限额。总开关默认关闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkAgentPolicy {
    pub enabled: bool,
    pub creations_per_source_per_hour: u32,
    pub creations_per_source_per_day: u32,
    pub max_live_agents: u64,
    pub messages_per_minute: u32,
    pub messages_per_day: u32,
}

impl NetworkAgentPolicy {
    /// 设计文档里的初始值：每个来源每小时建 5 个、每天 20 个，全站同时 500 个；
    /// 每个网络 Agent 每分钟发 20 条、每天 1000 条。
    pub const fn default_limits(enabled: bool) -> Self {
        Self {
            enabled,
            creations_per_source_per_hour: 5,
            creations_per_source_per_day: 20,
            max_live_agents: 500,
            messages_per_minute: 20,
            messages_per_day: 1_000,
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
    /// 所在的房间，先进的在前。
    pub rooms: Vec<NetworkAgentRoom>,
}

/// 网关代网络 Agent 收发时用的身份、Matrix 会话与签名种子。只在进程内传递，调试输出里不出现秘密。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSession {
    pub network_agent_id: NetworkAgentId,
    /// 网络 Agent 的合成主体：它发的正文归这个主体所有。
    pub principal_id: PrincipalId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub display_name: String,
    /// Agent 自己的 Matrix 用户，发言与验签都认它。
    pub agent_matrix_user_id: String,
    pub matrix_access_token: SecretValue,
    /// 实例签名种子（编码后）：替 Agent 签发言与状态。
    pub instance_signing_seed: SecretValue,
    pub rooms: Vec<NetworkAgentRoomRecord>,
    /// 实例的 Matrix 设备（`AR_<实例>`）；加密房间里的密钥发给这台设备。
    pub matrix_device_id: String,
    /// 第一次进加密房间的时刻；有值时它所有房间都改由 matrix-sdk 客户端收发。
    pub encrypted_since: Option<UtcMillis>,
}

/// 进加密房间要用的秘密（第 3 步）。只在进程内传递，调试输出里不出现内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentEncryptionSecrets {
    /// matrix-sdk 加密存储的口令：第一次要用时生成并封存，之后不变。
    pub store_passphrase: SecretValue,
    /// 服务器端密钥备份的恢复凭据；还没开启备份时为空。
    pub recovery_credential: Option<SecretValue>,
}

/// 能进的公开大厅：`name` 或 `slug` 都能交给创建时的 `room`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentLobby {
    pub name: String,
    pub slug: Option<String>,
    pub online_agent_count: u32,
    /// 省略 `room` 时进的就是这一间。
    pub default: bool,
}

/// 已停用、还没离开房间的网络 Agent。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentPendingExit {
    /// 会话打得开：先发“已离线”、离开这些房间，再记为已离开。
    Session(Box<NetworkAgentSession>),
    /// 秘密缺失或解不开（例如封存密钥换了），没法替它离开；调用方记日志后记为已离开，免得每轮都卡住。
    Unopenable(NetworkAgentId),
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

    /// 凭令牌取出网关代它收发要用的身份与 Matrix 会话，并记一次活动。
    fn session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>>;

    /// 发一条之前记一次：每分钟、每天各有上限，超了告诉 Agent 什么时候能再发。
    fn take_message_quota(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>>;

    /// 定时清理：停用闲置超过 30 天的与卡在创建中超过一小时的网络 Agent，返回停用了几个。
    fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>>;

    /// 已停用、还没离开房间的网络 Agent（运维停用、闲置停用，或自己停用时没离开成的），
    /// 一次最多 `limit` 个。不看总开关，也不记活动。
    fn pending_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>>;

    /// 离开了所有房间（或没法离开）之后记一笔，定时清理就不再找它。
    fn mark_rooms_left(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>>;

    /// 能进的公开大厅；总开关关着时回答“已关闭”。
    fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>>;

    /// 进加密房间要用的秘密：存储口令缺了就生成并封存；恢复凭据有就带上。
    /// 秘密在库里却解不开时报依赖不可用，绝不重新生成去覆盖：那会让已有的加密存储再也打不开。
    fn encryption_secrets(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentEncryptionSecrets>>;

    /// 开启服务器端密钥备份之前，先封存要用的恢复凭据；备份开好了凭据却没存下，就再也恢复不了。
    fn store_recovery_credential<'a>(
        &'a self,
        id: NetworkAgentId,
        credential: &'a SecretValue,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;
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
        self.store
            .record_room(
                id,
                &NetworkAgentRoomRecord {
                    catalog_id: room.catalog_id,
                    matrix_room_id: room.matrix_room_id.clone(),
                    joined_at: self.clock.now(),
                },
            )
            .await
            .map_err(repository)?;
        Ok((agent.agent.id(), room))
    }

    async fn me_internal(&self, token: &str) -> NetworkAgentResult<NetworkAgentView> {
        let record = self.authenticated(token).await?;
        let agent_id = record.agent_id.ok_or_else(internal)?;
        self.store
            .record_activity(record.id, self.clock.now())
            .await
            .map_err(repository)?;
        let mut rooms = Vec::new();
        for room in self.store.rooms(record.id).await.map_err(repository)? {
            let name = self
                .directory
                .find_catalog(room.catalog_id)
                .await
                .map_err(repository)?
                .map_or_else(String::new, |catalog| catalog.name().to_owned());
            rooms.push(NetworkAgentRoom {
                catalog_id: room.catalog_id,
                matrix_room_id: room.matrix_room_id,
                name,
            });
        }
        Ok(NetworkAgentView {
            network_agent_id: record.id,
            agent_id,
            display_name: record.display_name,
            created_at: record.created_at,
            rooms,
        })
    }

    async fn session_internal(&self, token: &str) -> NetworkAgentResult<NetworkAgentSession> {
        let record = self.authenticated(token).await?;
        let (Some(agent_id), Some(agent_instance_id)) = (record.agent_id, record.agent_instance_id)
        else {
            return Err(internal());
        };
        let matrix_access_token = self
            .open_secret(record.id, NetworkAgentSecretKind::MatrixAccessToken)
            .await?;
        let instance_signing_seed = self
            .open_secret(record.id, NetworkAgentSecretKind::InstanceSigningSeed)
            .await?;
        let rooms = self.store.rooms(record.id).await.map_err(repository)?;
        self.store
            .record_activity(record.id, self.clock.now())
            .await
            .map_err(repository)?;
        Ok(self.session_of(
            record,
            (agent_id, agent_instance_id),
            matrix_access_token,
            instance_signing_seed,
            rooms,
        ))
    }

    fn session_of(
        &self,
        record: NetworkAgentRecord,
        (agent_id, agent_instance_id): (AgentId, AgentInstanceId),
        matrix_access_token: SecretValue,
        instance_signing_seed: SecretValue,
        rooms: Vec<NetworkAgentRoomRecord>,
    ) -> NetworkAgentSession {
        NetworkAgentSession {
            network_agent_id: record.id,
            principal_id: record.principal_id,
            agent_id,
            agent_instance_id,
            display_name: record.display_name,
            agent_matrix_user_id: format!(
                "@{}:{}",
                MatrixAgentLocalpart::from_agent_id(agent_id).as_str(),
                self.matrix_server_name
            ),
            matrix_access_token,
            instance_signing_seed,
            rooms,
            matrix_device_id: crate::agents::instance_matrix_device_id(agent_instance_id),
            encrypted_since: record.encrypted_since,
        }
    }

    async fn encryption_secrets_internal(
        &self,
        id: NetworkAgentId,
    ) -> NetworkAgentResult<NetworkAgentEncryptionSecrets> {
        let store_passphrase = match self
            .read_secret(id, NetworkAgentSecretKind::MatrixStorePassphrase)
            .await?
        {
            OpenedSecret::Secret(secret) => secret,
            OpenedSecret::Missing => {
                let secret = self.secrets.generate().map_err(|_| dependency())?;
                self.put_text_secret(id, NetworkAgentSecretKind::MatrixStorePassphrase, &secret)
                    .await?;
                secret
            }
            OpenedSecret::Unsealable => return Err(dependency()),
        };
        let recovery_credential = match self
            .read_secret(id, NetworkAgentSecretKind::MatrixRecoveryKey)
            .await?
        {
            OpenedSecret::Secret(secret) => Some(secret),
            OpenedSecret::Missing => None,
            OpenedSecret::Unsealable => return Err(dependency()),
        };
        Ok(NetworkAgentEncryptionSecrets {
            store_passphrase,
            recovery_credential,
        })
    }

    async fn put_text_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        secret: &SecretValue,
    ) -> NetworkAgentResult<()> {
        let sealed = self
            .sealer
            .seal(id, kind, secret.expose().as_bytes())
            .map_err(|_| dependency())?;
        self.store
            .put_secret(id, kind, &sealed, self.clock.now())
            .await
            .map_err(repository)
    }

    /// 取出并解封一个秘密；缺失说明记录不完整，解不开说明密钥或密文出了问题，都不是令牌的错。
    async fn open_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> NetworkAgentResult<SecretValue> {
        match self.read_secret(id, kind).await? {
            OpenedSecret::Secret(secret) => Ok(secret),
            OpenedSecret::Missing => Err(internal()),
            OpenedSecret::Unsealable => Err(dependency()),
        }
    }

    /// 库不可用是错误；秘密缺失或解不开如实说明，由调用方决定怎么办。
    async fn read_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> NetworkAgentResult<OpenedSecret> {
        let Some(sealed) = self.store.find_secret(id, kind).await.map_err(repository)? else {
            return Ok(OpenedSecret::Missing);
        };
        Ok(self
            .sealer
            .open(id, kind, &sealed)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| SecretValue::new(text).ok())
            .map_or(OpenedSecret::Unsealable, OpenedSecret::Secret))
    }

    async fn disable_stale_internal(&self) -> NetworkAgentResult<usize> {
        let now = self.clock.now();
        let before = |millis: i64| UtcMillis::new(now.value() - millis).map_err(|_| internal());
        let cutoff = NetworkAgentStaleCutoff {
            idle_before: before(IDLE_LIFETIME_MILLIS)?,
            provisioning_before: before(PROVISIONING_TIMEOUT_MILLIS)?,
        };
        let disabled = self
            .store
            .disable_stale(cutoff, now, STALE_BATCH)
            .await
            .map_err(repository)?;
        Ok(disabled.len())
    }

    async fn pending_exits_internal(
        &self,
        limit: u32,
    ) -> NetworkAgentResult<Vec<NetworkAgentPendingExit>> {
        let records = self.store.pending_exits(limit).await.map_err(repository)?;
        let mut exits = Vec::with_capacity(records.len());
        for record in records {
            exits.push(self.pending_exit(record).await?);
        }
        Ok(exits)
    }

    async fn pending_exit(
        &self,
        record: NetworkAgentRecord,
    ) -> NetworkAgentResult<NetworkAgentPendingExit> {
        let id = record.id;
        let (Some(agent_id), Some(agent_instance_id)) = (record.agent_id, record.agent_instance_id)
        else {
            return Ok(NetworkAgentPendingExit::Unopenable(id));
        };
        let OpenedSecret::Secret(matrix_access_token) = self
            .read_secret(id, NetworkAgentSecretKind::MatrixAccessToken)
            .await?
        else {
            return Ok(NetworkAgentPendingExit::Unopenable(id));
        };
        let OpenedSecret::Secret(instance_signing_seed) = self
            .read_secret(id, NetworkAgentSecretKind::InstanceSigningSeed)
            .await?
        else {
            return Ok(NetworkAgentPendingExit::Unopenable(id));
        };
        let rooms = self.store.rooms(id).await.map_err(repository)?;
        Ok(NetworkAgentPendingExit::Session(Box::new(self.session_of(
            record,
            (agent_id, agent_instance_id),
            matrix_access_token,
            instance_signing_seed,
            rooms,
        ))))
    }

    async fn take_message_quota_internal(&self, id: NetworkAgentId) -> NetworkAgentResult<()> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        let agent = id.as_uuid().simple().to_string();
        for (window, limit, label) in [
            (60_000_u64, self.policy.messages_per_minute, "minute"),
            (86_400_000_u64, self.policy.messages_per_day, "day"),
        ] {
            self.take(&format!("send:{label}:{agent}"), window, limit)
                .await?;
        }
        Ok(())
    }

    /// 在一个固定窗口里记一次；到上限时告诉调用方窗口什么时候结束。
    async fn take(&self, bucket: &str, window: u64, limit: u32) -> NetworkAgentResult<()> {
        let policy = RateWindowPolicy {
            window: DurationMillis::new(window).map_err(|_| internal())?,
            limit,
        };
        match self
            .store
            .take(bucket, self.clock.now(), policy)
            .await
            .map_err(repository)?
        {
            RateWindowDecision::Allowed => Ok(()),
            RateWindowDecision::Limited { retry_at } => {
                Err(NetworkAgentFailure::rate_limited(retry_at))
            }
        }
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

    async fn public_lobbies_internal(&self) -> NetworkAgentResult<Vec<NetworkAgentLobby>> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        let lobbies = self
            .directory
            .list_public(&RoomDirectoryQuery::default())
            .await
            .map_err(repository)?;
        let default = lobbies
            .iter()
            .find(|entry| entry.catalog.slug().map(RoomSlug::as_str) == Some(DEFAULT_LOBBY_SLUG))
            .or_else(|| lobbies.first())
            .map(|entry| entry.catalog.id());
        Ok(lobbies
            .iter()
            .map(|entry| NetworkAgentLobby {
                name: entry.catalog.name().to_owned(),
                slug: entry.catalog.slug().map(|slug| slug.as_str().to_owned()),
                online_agent_count: entry.online_agent_count,
                default: Some(entry.catalog.id()) == default,
            })
            .collect())
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
            self.take(&format!("create:{label}:{source}"), window, limit)
                .await?;
        }
        Ok(())
    }

    /// 进公开大厅。大厅正在准备新房间时按服务器给的时间等一会儿再试。
    async fn enter(
        &self,
        actor: &AuthenticatedDevice,
        agent_id: AgentId,
        agent_instance_id: AgentInstanceId,
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

    fn session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
        Box::pin(self.session_internal(token))
    }

    fn take_message_quota(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        Box::pin(self.take_message_quota_internal(id))
    }

    fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>> {
        Box::pin(self.disable_stale_internal())
    }

    fn pending_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>> {
        Box::pin(self.pending_exits_internal(limit))
    }

    fn mark_rooms_left(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        Box::pin(async move {
            self.store
                .mark_rooms_left(id, self.clock.now())
                .await
                .map_err(repository)
        })
    }

    fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>> {
        Box::pin(self.public_lobbies_internal())
    }

    fn encryption_secrets(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentEncryptionSecrets>> {
        Box::pin(self.encryption_secrets_internal(id))
    }

    fn store_recovery_credential<'a>(
        &'a self,
        id: NetworkAgentId,
        credential: &'a SecretValue,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.put_text_secret(id, NetworkAgentSecretKind::MatrixRecoveryKey, credential))
    }
}

/// 读一个封存秘密的结果。
enum OpenedSecret {
    Secret(SecretValue),
    Missing,
    Unsealable,
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
