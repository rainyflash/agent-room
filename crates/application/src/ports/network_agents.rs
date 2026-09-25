//! 只凭网络接入的 Agent（ADR 0010）：服务器替它保管的身份、封存的秘密和限流窗口。

use std::fmt;

use agent_room_domain::{
    devices::Device,
    ids::{
        AgentId, AgentInstanceId, DeviceId, MessageId, MessageSubmissionId, NetworkAgentId,
        PrincipalId, RoomCatalogId,
    },
    network_agents::NetworkAgentStatus,
    rooms::MatrixRoomReference,
    time::{DurationMillis, UtcMillis},
};
use serde_json::Value;

use crate::{
    persistence::RepositoryResult,
    ports::{
        MatrixAcceptedEvent, MatrixEvent, MatrixEventId, MatrixResult, MatrixRoomId,
        MatrixStateEvent, MatrixSyncBatch, MatrixSyncToken, MatrixTransactionId, PortFuture,
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
    /// matrix-sdk 加密存储的口令（第 3 步：进加密房间的网络 Agent 才有）。
    MatrixStorePassphrase,
    /// 服务器端密钥备份的恢复密钥：存储丢失时换设备重建身份与房间密钥。
    MatrixRecoveryKey,
    /// 加密房间里发言正文的根密钥，与本机 Bridge 的正文保护一样。
    MessageContentRootKey,
}

impl NetworkAgentSecretKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeviceSigningSeed => "device_signing_seed",
            Self::InstanceSigningSeed => "instance_signing_seed",
            Self::MatrixAccessToken => "matrix_access_token",
            Self::MatrixStorePassphrase => "matrix_store_passphrase",
            Self::MatrixRecoveryKey => "matrix_recovery_key",
            Self::MessageContentRootKey => "message_content_root_key",
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
    /// 实例现在的 Matrix 设备；加密存储丢了重建时会换成新的设备 ID。
    pub matrix_device_id: Option<String>,
    pub display_name: String,
    pub status: NetworkAgentStatus,
    pub created_at: UtcMillis,
    pub last_active_at: UtcMillis,
    /// 第一次进加密房间的时刻；有值时它所有房间都改由 matrix-sdk 客户端收发。
    pub encrypted_since: Option<UtcMillis>,
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

    fn find(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<NetworkAgentRecord>>>;

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

    /// 停用闲置太久的、以及卡在创建中太久的网络 Agent，一次最多 `limit` 个；返回停用了哪些。
    fn disable_stale(
        &self,
        cutoff: NetworkAgentStaleCutoff,
        at: UtcMillis,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentId>>>;

    /// 已停用、还没离开房间的网络 Agent，最早停用的在前。没建好实例就停用的不在其中：
    /// 它们从没进过房间，停用时就记为已离开。
    fn pending_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<NetworkAgentRecord>>>;

    /// 存一个封存的秘密：没有就新增，有就替换（例如第一次进加密房间时生成的存储口令）。
    fn put_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        sealed: &SealedSecret,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 记下第一次进加密房间的时刻；已经记过的不改，返回记下的那一刻。
    fn mark_encrypted(
        &self,
        id: NetworkAgentId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<UtcMillis>>;

    /// 记下已经离开了所有房间；只对已停用的生效，记过的不改。
    fn mark_rooms_left(
        &self,
        id: NetworkAgentId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>>;
}

/// 定时清理的界线：最后活动早于 `idle_before` 的生效中网络 Agent，以及创建早于
/// `provisioning_before` 却还没建好的，都停用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkAgentStaleCutoff {
    pub idle_before: UtcMillis,
    pub provisioning_before: UtcMillis,
}

/// 网页标注“网络 Agent”用：给一批 Agent ID，找出其中属于网络 Agent 的。
/// 停用的也算，所以同一个 Agent 的答案永远不变，调用方可以一直缓存。
pub trait NetworkAgentLookup: Send + Sync {
    fn network_agent_ids<'a>(
        &'a self,
        candidates: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<AgentId>>>;
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

    /// 以 Agent 自己的身份发一条消息事件；同一事务 ID 重发不会重复。
    fn send_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>>;

    /// 以 Agent 自己的身份写一条房间状态（在线状态用）。
    fn send_state_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>>;

    /// 离开房间；已经不在里面也算成功。
    fn leave<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

/// 网络 Agent 发出的一次提交是哪一种。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentSubmissionKind {
    Preview,
    Replace,
    Redact,
}

impl NetworkAgentSubmissionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Replace => "replace",
            Self::Redact => "redact",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "preview" => Some(Self::Preview),
            "replace" => Some(Self::Replace),
            "redact" => Some(Self::Redact),
            _ => None,
        }
    }
}

/// 一次提交走到哪一步：占住、不知道 Matrix 收没收到、Matrix 已收下、正文已绑定到事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentSubmissionState {
    Claimed,
    SubmitUnknown,
    Accepted,
    Bound,
}

impl NetworkAgentSubmissionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::SubmitUnknown => "submit_unknown",
            Self::Accepted => "accepted",
            Self::Bound => "bound",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claimed" => Some(Self::Claimed),
            "submit_unknown" => Some(Self::SubmitUnknown),
            "accepted" => Some(Self::Accepted),
            "bound" => Some(Self::Bound),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSubmissionClaim {
    pub submission_id: MessageSubmissionId,
    pub kind: NetworkAgentSubmissionKind,
    /// 这次提交内容的摘要：同一个提交 ID 换了内容就是冲突。
    pub fingerprint: [u8; 32],
    pub transaction_id: MatrixTransactionId,
    pub claimed_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSubmissionRecord {
    pub submission_id: MessageSubmissionId,
    pub kind: NetworkAgentSubmissionKind,
    pub fingerprint: [u8; 32],
    pub transaction_id: MatrixTransactionId,
    pub state: NetworkAgentSubmissionState,
    pub event_id: Option<MatrixEventId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentSubmissionClaimOutcome {
    Created(NetworkAgentSubmissionRecord),
    Existing(NetworkAgentSubmissionRecord),
}

/// 网络 Agent 发言的幂等记录，语义与本机 Bridge 的提交记录一致，只是按网络 Agent 分开存。
pub trait NetworkAgentSubmissionStore: Send + Sync {
    /// 占住一个提交 ID；已经有了就原样返回，内容对不上时返回冲突。
    fn claim<'a>(
        &'a self,
        id: NetworkAgentId,
        claim: &'a NetworkAgentSubmissionClaim,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentSubmissionClaimOutcome>>;

    /// 发出去但不知道 Matrix 收没收到（只从“占住”转过来）。
    fn mark_submit_unknown(
        &self,
        id: NetworkAgentId,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentSubmissionRecord>>;

    /// Matrix 收下了，记下事件 ID；已经绑定的不回退。
    fn mark_accepted<'a>(
        &'a self,
        id: NetworkAgentId,
        submission_id: MessageSubmissionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<NetworkAgentSubmissionRecord>>;

    /// 正文已绑定到事件，这次提交完成。
    fn mark_bound(
        &self,
        id: NetworkAgentId,
        submission_id: MessageSubmissionId,
    ) -> PortFuture<'_, RepositoryResult<NetworkAgentSubmissionRecord>>;

    /// 同步时看到自己发的事件：按事务 ID 找到那次提交并记为已收下。
    fn observe_transaction<'a>(
        &'a self,
        id: NetworkAgentId,
        transaction_id: &'a MatrixTransactionId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, RepositoryResult<Option<NetworkAgentSubmissionRecord>>>;
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
