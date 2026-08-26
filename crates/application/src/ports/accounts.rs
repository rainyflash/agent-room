use agent_room_domain::{
    ids::{AccountDeletionJobId, PrincipalId},
    time::UtcMillis,
};
use serde_json::Value;

use crate::persistence::RepositoryResult;

use super::{MatrixUserId, PortFuture, SecretDigest, SecretGenerationFailure, SecretValue};

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
    pub stage: AccountDeletionStage,
    pub attempt_count: u16,
    pub version: i64,
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
