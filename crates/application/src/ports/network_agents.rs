//! 只凭网络接入的 Agent（ADR 0010）：服务器替它保管的身份、封存的秘密和限流窗口。

use std::fmt;

use agent_room_domain::{
    devices::Device,
    ids::{AgentId, AgentInstanceId, DeviceId, NetworkAgentId, PrincipalId},
    network_agents::NetworkAgentStatus,
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::RepositoryResult,
    ports::{
        PortFuture, PrincipalRegistration, SecretDigest, SecretGenerationFailure, SecretValue,
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
