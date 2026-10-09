use agent_room_domain::{
    ids::{AccountDeletionJobId, PrincipalId},
    time::UtcMillis,
};
use serde_json::Value;

use crate::persistence::RepositoryResult;

use super::{
    MatrixUserId, OidcResult, PortFuture, SecretDigest, SecretGenerationFailure, SecretValue,
};

#[derive(Debug, Clone, PartialEq)]
pub struct AccountExportSnapshot {
    pub schema_version: u16,
    pub generated_at: UtcMillis,
    pub data: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDeletionStage {
    Queued,
    FederatedDeactivation,
    LocalErasure,
    RetryScheduled,
    Completed,
}

impl AccountDeletionStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::FederatedDeactivation => "federated_deactivation",
            Self::LocalErasure => "local_erasure",
            Self::RetryScheduled => "retry_scheduled",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeletionRequest {
    pub job_id: AccountDeletionJobId,
    pub principal_id: PrincipalId,
    pub matrix_user_id: MatrixUserId,
    pub receipt_digest: SecretDigest,
    pub requested_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeletionStatus {
    pub job_id: AccountDeletionJobId,
    pub stage: AccountDeletionStage,
    pub attempt_count: u16,
    pub requested_at: UtcMillis,
    pub updated_at: UtcMillis,
    pub retry_at: Option<UtcMillis>,
    pub completed_at: Option<UtcMillis>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountDeletionRequestOutcome {
    Created(AccountDeletionStatus),
    Existing(AccountDeletionStatus),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeletionClaim {
    pub job_id: AccountDeletionJobId,
    pub principal_id: PrincipalId,
    pub matrix_user_id: MatrixUserId,
    pub sign_in_account: SignInAccount,
    pub stage: AccountDeletionStage,
    pub attempt_count: u16,
    pub version: i64,
}

/// 登录服务里的账户：OIDC 签发方和用户号（Keycloak 里用户号就是用户 ID）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInAccount {
    pub issuer: String,
    pub subject: String,
}

/// 删除账户时删掉登录服务里的账户：邮箱、昵称和密码散列都存在那里。
///
/// 实现只删本部署签发方的账户，别的签发方（比如网络 Agent 的 `urn:agent-room:network-agent`）
/// 当作没有可删的。账户已经不在也算成功：任务重试、备份恢复后重放都会再调一次。
pub trait SignInAccountRemoval: Send + Sync {
    fn remove<'a>(&'a self, account: &'a SignInAccount) -> PortFuture<'a, OidcResult<()>>;
}

pub trait AccountDeletionReceiptIssuer: Send + Sync {
    /// 为同一幂等任务稳定派生同一不可预测回执，响应丢失后可以安全重放。
    ///
    /// # Errors
    ///
    /// 密码学派生器无法安全产生回执时返回错误。
    fn issue(&self, job_id: AccountDeletionJobId) -> Result<SecretValue, SecretGenerationFailure>;

    fn digest(&self, value: &str) -> SecretDigest;
}

pub trait AccountDeletionRepository: Send + Sync {
    fn export(
        &self,
        principal_id: PrincipalId,
        generated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AccountExportSnapshot>>>;

    /// 请求、主体进入 deleting、全部本地凭据撤销必须属于同一事务。
    fn request<'a>(
        &'a self,
        request: &'a AccountDeletionRequest,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionRequestOutcome>>;

    fn find_by_receipt<'a>(
        &'a self,
        receipt_digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<AccountDeletionStatus>>>;

    /// 使用有界租约和 `SKIP LOCKED` 抢占一个到期任务，多副本之间不得重复执行外部副作用。
    fn claim_due(
        &self,
        now: UtcMillis,
        lease_expires_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AccountDeletionClaim>>>;

    fn record_federated_deactivation<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionClaim>>;

    fn schedule_retry<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        failure_code: &'a str,
        retry_at: UtcMillis,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    /// 匿名化资料、撤销所有本地授权、归档仅有该主体所有的资源，并把内容送入回收队列。
    fn finalize_local<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionStatus>>;
}

/// 服务器替账户保管的密钥存储钥匙（ADR 0011），封存后入库。每个主体一份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEncryptionKey {
    /// 账户数据里那把密钥存储钥匙的 ID。
    pub key_id: String,
    pub sealed: super::SealedSecret,
}

pub trait AccountEncryptionKeyRepository: Send + Sync {
    fn find_encryption_key(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<StoredEncryptionKey>>>;

    /// 存下或覆盖这个主体的钥匙。
    fn put_encryption_key<'a>(
        &'a self,
        principal_id: PrincipalId,
        key: &'a StoredEncryptionKey,
        updated_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

/// AES-256-GCM 封存账户的密钥存储钥匙；附加数据绑定主体，密文挪给别的账户打不开。
pub trait AccountEncryptionKeySealer: Send + Sync {
    /// # Errors
    ///
    /// 封存密钥没有配置时返回错误。
    fn seal(
        &self,
        principal_id: PrincipalId,
        plaintext: &[u8],
    ) -> Result<super::SealedSecret, super::SecretSealingFailure>;

    /// # Errors
    ///
    /// 封存密钥没有配置、密文被改过或不属于这个主体时返回错误。
    fn open(
        &self,
        principal_id: PrincipalId,
        sealed: &super::SealedSecret,
    ) -> Result<Vec<u8>, super::SecretSealingFailure>;
}
