//! 只凭网络接入的 Agent（ADR 0010）：服务器替它保管的身份、封存的秘密和限流窗口。

use std::fmt;

use agent_room_domain::{
    devices::Device,
    ids::{
        AgentId, AgentInstanceId, DeviceId, MessageId, NetworkAgentId, PrincipalId, RoomCatalogId,
    },
    network_agents::NetworkAgentStatus,
    rooms::MatrixRoomReference,
    time::{DurationMillis, UtcMillis},
};
use serde_json::Value;

use crate::{
    persistence::RepositoryResult,
    ports::{
        MatrixEventId, MatrixResult, MatrixRoomId, MatrixSyncBatch, MatrixSyncToken, PortFuture,
        PrincipalRegistration, SecretDigest, SecretGenerationFailure, SecretValue,
    },
};

/// 服务器替网络 Agent 保管的秘密。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentSecretKind {
    /// 网络设备的签名种子；网络 Agent 不走设备签名的接口，留着只为以后轮换设备时用。
    DeviceSigningSeed,
    /// 实例签名种子：替 Agent 签发言与状态。
    InstanceSigningSeed,
    /// Agent 的 Matrix 访问令牌：替它同步与发送。
    MatrixAccessToken,
}

impl NetworkAgentSecretKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeviceSigningSeed => "device_signing_seed",
            Self::InstanceSigningSeed => "instance_signing_seed",
            Self::MatrixAccessToken => "matrix_access_token",
        }
    }
}

/// 用部署配置里的封存密钥加密后的秘密；库里只存这个，调试输出里不出现内容。
#[derive(Clone, PartialEq, Eq)]
pub struct SealedSecret {
    pub key_version: u16,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for SealedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedSecret")
            .field("key_version", &self.key_version)
            .field(
                "bytes",
                &format_args!("[{} 字节，已封存]", self.bytes.len()),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSealingFailure {
    /// 封存密钥没有配置或读不到。
    Unavailable,
    /// 密文被改过、密钥版本不对，或者明文不是预期的格式。
    Corrupt,
}

/// AES-256-GCM 封存；附加数据绑定网络 Agent 与秘密种类，密文不能挪给别的 Agent 或别的用途。
pub trait NetworkAgentSecretSealer: Send + Sync {
    /// # Errors
    ///
    /// 封存密钥不可用时返回错误。
    fn seal(
        &self,
        agent: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure>;

    /// # Errors
    ///
    /// 封存密钥不可用、密文被改过或不属于这个 Agent 与种类时返回错误。
    fn open(
        &self,
        agent: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure>;
}

/// 一个事务里写入主体、网络设备、网络 Agent（provisioning）和封存的签名种子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentProvisioning {
    pub id: NetworkAgentId,
    pub principal: PrincipalRegistration,
    /// 已验证、平台为 network 的设备。
    pub device: Device,
    pub device_label: String,
    pub token_digest: SecretDigest,
    pub display_name: String,
    pub source_digest: [u8; 32],
    pub secrets: Vec<(NetworkAgentSecretKind, SealedSecret)>,
    pub created_at: UtcMillis,
}

/// Agent 与实例建好之后生效，同时保存封存的 Matrix 访问令牌。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentActivation {
    pub id: NetworkAgentId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub matrix_access_token: SealedSecret,
    pub activated_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentRecord {
    pub id: NetworkAgentId,
    pub principal_id: PrincipalId,
    pub device_id: DeviceId,
    pub agent_id: Option<AgentId>,
    pub agent_instance_id: Option<AgentInstanceId>,
    pub display_name: String,
    pub status: NetworkAgentStatus,
    pub created_at: UtcMillis,
    pub last_active_at: UtcMillis,
}

/// 固定窗口：窗口里最多 `limit` 次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateWindowPolicy {
    pub window: DurationMillis,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateWindowDecision {
    Allowed,
    Limited { retry_at: UtcMillis },
}

/// 写入网络 Agent 时名字撞上了还没停用的同名者（不分大小写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentBeginOutcome {
    Created,
    NameTaken,
}

pub trait NetworkAgentStore: Send + Sync {
    fn begin(
        &self,
        provisioning: &NetworkAgentProvisioning,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentBeginOutcome>>;

    fn activate(&self, activation: &NetworkAgentActivation)
    -> PortFuture<'_, RepositoryResult<()>>;

    fn find_by_token<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<NetworkAgentRecord>>>;

    fn find_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedSecret>>>;

    /// 还没停用的网络 Agent（含正在创建的），用于全站上限。
    fn count_live(&self) -> PortFuture<'_, RepositoryResult<u64>>;

    fn record_activity(
        &self,
        id: NetworkAgentId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 停用：令牌作废。已经停用时也算成功。
    fn disable(&self, id: NetworkAgentId, at: UtcMillis) -> PortFuture<'_, RepositoryResult<()>>;

    /// 在固定窗口里记一次；已经到上限时不记，返回下次可以的时间。
    fn take<'a>(
        &'a self,
        bucket: &'a str,
        now: UtcMillis,
        policy: RateWindowPolicy,
    ) -> PortFuture<'a, RepositoryResult<RateWindowDecision>>;

    /// 记下网络 Agent 进了哪个房间；重复记同一个房间不改最初的时间。
    fn record_room<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a NetworkAgentRoomRecord,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 进过的房间，先进的在前。
    fn rooms(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentRoomRecord>>>;
}

/// 网络 Agent 所在的一个房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentRoomRecord {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub joined_at: UtcMillis,
}

/// 同步到的一条变化，按时间线顺序写进收件箱。
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkAgentInboxChange {
    /// 验签通过的新消息；同一事件重复同步到时只留第一次。
    Message(NetworkAgentInboxMessage),
    /// 作者改了还没确认的那条：用新字段覆盖预览里的同名字段。
    Replace {
        room_id: MatrixRoomId,
        message_id: MessageId,
        actor_key: String,
        patch: Value,
    },
    /// 作者撤回了还没确认的那条：直接从收件箱拿掉。
    Redact {
        room_id: MatrixRoomId,
        message_id: MessageId,
        actor_key: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkAgentInboxMessage {
    pub event_id: MatrixEventId,
    pub room_id: MatrixRoomId,
    pub message_id: MessageId,
    /// 作者的稳定标识（Agent ID 或 `human:<Matrix 用户>`），修订只认同一作者。
    pub actor_key: String,
    /// 与 CLI、MCP 看到的形状一致的消息预览。
    pub preview: Value,
}

/// 一次同步的结果：从 `expected_sync_token` 同步到 `next_sync_token`。
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkAgentInboxAppend {
    pub id: NetworkAgentId,
    /// 同步开始时的位置；和库里的对不上说明另一次同步已经写过，这次作废。
    pub expected_sync_token: Option<MatrixSyncToken>,
    pub next_sync_token: MatrixSyncToken,
    pub changes: Vec<NetworkAgentInboxChange>,
    pub received_at: UtcMillis,
    /// 最多保留这么多条没确认的；再多就丢掉最早的并计数。
    pub capacity: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentInboxAppendOutcome {
    Applied {
        appended: u32,
    },
    /// 位置已被另一次同步推进，这批没写。
    Stale,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkAgentInboxEntry {
    pub sequence: u64,
    pub event_id: MatrixEventId,
    pub preview: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkAgentInboxPage {
    pub sync_token: Option<MatrixSyncToken>,
    /// 最早的在前。
    pub entries: Vec<NetworkAgentInboxEntry>,
    /// 还没确认的总数（可能多于这一页）。
    pub pending: u64,
    /// 收件箱满了丢掉的条数，下次确认后清零。
    pub dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentAckOutcome {
    /// 确认到这一条为止，返回还剩多少条没确认。
    Acknowledged { pending: u64 },
    /// 收件箱里没有这一条：可能早就确认过了。
    NotPending { pending: u64 },
}

/// 网络 Agent 的收件箱：只有显式确认才往前走。
pub trait NetworkAgentInboxStore: Send + Sync {
    /// 读还没确认的前 `limit` 条，以及当前同步位置。
    fn pending(
        &self,
        id: NetworkAgentId,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentInboxPage>>;

    /// 原子写入一次同步的结果并推进同步位置。
    fn append<'a>(
        &'a self,
        append: &'a NetworkAgentInboxAppend,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentInboxAppendOutcome>>;

    /// 确认到这一条（含）为止，确认过的从收件箱删掉。
    fn acknowledge<'a>(
        &'a self,
        id: NetworkAgentId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentAckOutcome>>;
}

/// 一次同步请求。服务器最多等 `timeout_millis` 就返回，哪怕没有新消息；0 表示立即返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSyncRequest {
    pub since: Option<MatrixSyncToken>,
    pub timeout_millis: u64,
    /// 每个房间最多带回多少条时间线事件；第一次同步只带最近几条做上下文。
    pub timeline_limit: u16,
}

/// 用网络 Agent 自己的 Matrix 会话（访问令牌由服务器封存保管）访问 Matrix。
pub trait NetworkAgentMatrixGateway: Send + Sync {
    /// 只同步 Agent Room 的消息事件，不同步状态、回执与在线信息。
    fn sync<'a>(
        &'a self,
        access_token: &'a SecretValue,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>>;
}

/// 服务器生成的 Ed25519 签名密钥：种子只在封存前短暂存在。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSigningKey {
    pub encoded_seed: SecretValue,
    pub public_key: [u8; 32],
}

pub trait NetworkAgentKeyFactory: Send + Sync {
    /// # Errors
    ///
    /// 系统安全随机源不可用时返回错误。
    fn generate(&self) -> Result<GeneratedSigningKey, SecretGenerationFailure>;
}

/// 大厅正在准备新房间时等一会儿再进；应用层不绑定运行时。
pub trait NetworkAgentPause: Send + Sync {
    fn until(&self, at: UtcMillis) -> PortFuture<'_, ()>;
}
