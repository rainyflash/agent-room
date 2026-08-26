use std::sync::Arc;

use agent_room_domain::{
    ids::AccountDeletionJobId,
    time::{DurationMillis, UtcMillis},
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AccountDeletionReceiptIssuer, AccountDeletionRepository, AccountDeletionRequest,
        AccountDeletionRequestOutcome, AccountDeletionStatus, AccountExportSnapshot, Clock,
        MatrixAccountLifecycleGateway, MatrixFailure, MatrixFailureKind, MatrixUserId, PortFuture,
        SecretValue,
    },
};

const DELETION_CONFIRMATION: &str = "DELETE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportAccount {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestAccountDeletion {
    pub actor: AuthenticatedPrincipal,
    pub job_id: AccountDeletionJobId,
    pub confirmation: String,
    pub federation_residual_acknowledged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayAccountDeletion {
    pub job_id: AccountDeletionJobId,
    pub confirmation: String,
    pub federation_residual_acknowledged: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct InspectAccountDeletion {
    pub receipt: SecretValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedAccountDeletion {
    pub receipt: SecretValue,
    pub status: AccountDeletionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountLifecycleFailureKind {
    InvalidRequest,
    Forbidden,
    NotFound,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountLifecycleFailure {
    operation: &'static str,
    kind: AccountLifecycleFailureKind,
}

impl AccountLifecycleFailure {
    const fn new(operation: &'static str, kind: AccountLifecycleFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AccountLifecycleFailureKind {
        self.kind
    }
}

pub type AccountLifecycleResult<T> = Result<T, AccountLifecycleFailure>;

pub trait AccountLifecycleUseCases: Send + Sync {
    fn export(
        &self,
        request: ExportAccount,
    ) -> PortFuture<'_, AccountLifecycleResult<AccountExportSnapshot>>;

    fn request_deletion(
        &self,
        request: RequestAccountDeletion,
    ) -> PortFuture<'_, AccountLifecycleResult<StartedAccountDeletion>>;

    fn replay_deletion(
        &self,
        request: ReplayAccountDeletion,
    ) -> PortFuture<'_, AccountLifecycleResult<StartedAccountDeletion>>;

    fn inspect_deletion(
        &self,
        request: InspectAccountDeletion,
    ) -> PortFuture<'_, AccountLifecycleResult<AccountDeletionStatus>>;
}

pub struct AccountLifecycleDependencies {
    pub repository: Arc<dyn AccountDeletionRepository>,
    pub receipts: Arc<dyn AccountDeletionReceiptIssuer>,
    pub clock: Arc<dyn Clock>,
}

pub struct AccountLifecycleService {
    repository: Arc<dyn AccountDeletionRepository>,
    receipts: Arc<dyn AccountDeletionReceiptIssuer>,
    clock: Arc<dyn Clock>,
}

impl AccountLifecycleService {
    pub fn new(dependencies: AccountLifecycleDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            receipts: dependencies.receipts,
            clock: dependencies.clock,
        }
    }

    async fn export_internal(
        &self,
        request: ExportAccount,
    ) -> AccountLifecycleResult<AccountExportSnapshot> {
        const OPERATION: &str = "account.export";
        let now = self.clock.now();
        require_active(&request.actor, now, false, OPERATION)?;
        self.repository
            .export(request.actor.principal_id, now)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
            .ok_or_else(|| {
                AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::NotFound)
            })
    }

    async fn request_deletion_internal(
        &self,
        request: RequestAccountDeletion,
    ) -> AccountLifecycleResult<StartedAccountDeletion> {
        const OPERATION: &str = "account.request_deletion";
        let now = self.clock.now();
        require_active(&request.actor, now, true, OPERATION)?;
        validate_deletion_confirmation(
            &request.confirmation,
            request.federation_residual_acknowledged,
            OPERATION,
        )?;
        let matrix_user_id =
            MatrixUserId::new(request.actor.matrix_user_id.clone()).map_err(|_| {
                AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::Internal)
            })?;
        let receipt = self.receipts.issue(request.job_id).map_err(|_| {
            AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::Internal)
        })?;
        let deletion = AccountDeletionRequest {
            job_id: request.job_id,
            principal_id: request.actor.principal_id,
            matrix_user_id,
            receipt_digest: self.receipts.digest(receipt.expose()),
            requested_at: now,
        };
        match self
            .repository
            .request(&deletion)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        {
            AccountDeletionRequestOutcome::Created(status) => {
                Ok(StartedAccountDeletion { receipt, status })
            }
            AccountDeletionRequestOutcome::Existing(status) if status.job_id == request.job_id => {
                Ok(StartedAccountDeletion { receipt, status })
            }
            AccountDeletionRequestOutcome::Existing(_) => Err(AccountLifecycleFailure::new(
                OPERATION,
                AccountLifecycleFailureKind::Conflict,
            )),
        }
    }

    async fn inspect_deletion_internal(
        &self,
        request: InspectAccountDeletion,
    ) -> AccountLifecycleResult<AccountDeletionStatus> {
        const OPERATION: &str = "account.inspect_deletion";
        let digest = self.receipts.digest(request.receipt.expose());
        self.repository
            .find_by_receipt(&digest)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
            .ok_or_else(|| {
                AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::NotFound)
            })
    }

    async fn replay_deletion_internal(
        &self,
        request: ReplayAccountDeletion,
    ) -> AccountLifecycleResult<StartedAccountDeletion> {
        const OPERATION: &str = "account.replay_deletion";
        validate_deletion_confirmation(
            &request.confirmation,
            request.federation_residual_acknowledged,
            OPERATION,
        )?;
        let receipt = self.receipts.issue(request.job_id).map_err(|_| {
            AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::Internal)
        })?;
        let digest = self.receipts.digest(receipt.expose());
        let status = self
            .repository
            .find_by_receipt(&digest)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
            .filter(|status| status.job_id == request.job_id)
            .ok_or_else(|| {
                AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::NotFound)
            })?;
        Ok(StartedAccountDeletion { receipt, status })
    }
}

impl AccountLifecycleUseCases for AccountLifecycleService {
    fn export(
        &self,
        request: ExportAccount,
    ) -> PortFuture<'_, AccountLifecycleResult<AccountExportSnapshot>> {
        Box::pin(self.export_internal(request))
    }

    fn request_deletion(
        &self,
        request: RequestAccountDeletion,
    ) -> PortFuture<'_, AccountLifecycleResult<StartedAccountDeletion>> {
        Box::pin(self.request_deletion_internal(request))
    }

    fn replay_deletion(
        &self,
        request: ReplayAccountDeletion,
    ) -> PortFuture<'_, AccountLifecycleResult<StartedAccountDeletion>> {
        Box::pin(self.replay_deletion_internal(request))
    }

    fn inspect_deletion(
        &self,
        request: InspectAccountDeletion,
    ) -> PortFuture<'_, AccountLifecycleResult<AccountDeletionStatus>> {
        Box::pin(self.inspect_deletion_internal(request))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDeletionWorkerOutcome {
    Idle,
    Completed(AccountDeletionJobId),
    Retrying(AccountDeletionJobId),
}

pub struct AccountDeletionWorkerDependencies {
    pub repository: Arc<dyn AccountDeletionRepository>,
    pub matrix: Arc<dyn MatrixAccountLifecycleGateway>,
    pub clock: Arc<dyn Clock>,
    pub lease_duration: DurationMillis,
    pub initial_retry_delay: DurationMillis,
    pub maximum_retry_delay: DurationMillis,
}

pub struct AccountDeletionWorker {
    repository: Arc<dyn AccountDeletionRepository>,
    matrix: Arc<dyn MatrixAccountLifecycleGateway>,
    clock: Arc<dyn Clock>,
    lease_duration: DurationMillis,
    initial_retry_delay: DurationMillis,
    maximum_retry_delay: DurationMillis,
}

impl AccountDeletionWorker {
    pub fn new(dependencies: AccountDeletionWorkerDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
            lease_duration: dependencies.lease_duration,
            initial_retry_delay: dependencies.initial_retry_delay,
            maximum_retry_delay: dependencies.maximum_retry_delay,
        }
    }

    /// 执行至多一个到期删除任务。
    ///
    /// # Errors
    ///
    /// 持久化、Matrix 擦除或时间计算无法安全完成时返回失败。
    pub async fn run_once(&self) -> AccountLifecycleResult<AccountDeletionWorkerOutcome> {
        const OPERATION: &str = "account.worker.run_once";
        let now = self.clock.now();
        let lease_expires_at = now.checked_add(self.lease_duration).map_err(|_| {
            AccountLifecycleFailure::new(OPERATION, AccountLifecycleFailureKind::Internal)
        })?;
        let Some(mut claim) = self
            .repository
            .claim_due(now, lease_expires_at)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        else {
            return Ok(AccountDeletionWorkerOutcome::Idle);
        };

        if matches!(
            claim.stage,
            crate::ports::AccountDeletionStage::FederatedDeactivation
        ) {
            if let Err(failure) = self
                .matrix
                .deactivate_and_erase(&claim.matrix_user_id)
                .await
            {
                let retry_at = now
                    .checked_add(self.retry_delay(claim.attempt_count))
                    .map_err(|_| {
                        AccountLifecycleFailure::new(
                            OPERATION,
                            AccountLifecycleFailureKind::Internal,
                        )
                    })?;
                self.repository
                    .schedule_retry(&claim, matrix_failure_code(failure), retry_at, now)
                    .await
                    .map_err(|error| repository_failure(OPERATION, &error))?;
                return Ok(AccountDeletionWorkerOutcome::Retrying(claim.job_id));
            }
            claim = self
                .repository
                .record_federated_deactivation(&claim, now)
                .await
                .map_err(|error| repository_failure(OPERATION, &error))?;
        }

        self.repository
            .finalize_local(&claim, now)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        Ok(AccountDeletionWorkerOutcome::Completed(claim.job_id))
    }

    fn retry_delay(&self, completed_attempts: u16) -> DurationMillis {
        let shift = u32::from(completed_attempts.saturating_sub(1)).min(20);
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let millis = self
            .initial_retry_delay
            .value()
            .saturating_mul(multiplier)
            .min(self.maximum_retry_delay.value());
        DurationMillis::new(millis).expect("账户删除退避配置保证延迟非零")
    }
}

fn require_active(
    actor: &AuthenticatedPrincipal,
    now: UtcMillis,
    recent_required: bool,
    operation: &'static str,
) -> AccountLifecycleResult<()> {
    if now >= actor.expires_at {
        return Err(AccountLifecycleFailure::new(
            operation,
            AccountLifecycleFailureKind::Forbidden,
        ));
    }
    if recent_required && !actor.recently_authenticated {
        return Err(AccountLifecycleFailure::new(
            operation,
            AccountLifecycleFailureKind::Forbidden,
        ));
    }
    Ok(())
}

fn validate_deletion_confirmation(
    confirmation: &str,
    federation_residual_acknowledged: bool,
    operation: &'static str,
) -> AccountLifecycleResult<()> {
    if confirmation != DELETION_CONFIRMATION || !federation_residual_acknowledged {
        return Err(AccountLifecycleFailure::new(
            operation,
            AccountLifecycleFailureKind::InvalidRequest,
        ));
    }
    Ok(())
}

fn repository_failure(
    operation: &'static str,
    failure: &RepositoryError,
) -> AccountLifecycleFailure {
    let kind = match failure.kind() {
        RepositoryErrorKind::Conflict => AccountLifecycleFailureKind::Conflict,
        RepositoryErrorKind::Forbidden => AccountLifecycleFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => AccountLifecycleFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => AccountLifecycleFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            AccountLifecycleFailureKind::Internal
        }
    };
    AccountLifecycleFailure::new(operation, kind)
}

fn matrix_failure_code(failure: MatrixFailure) -> &'static str {
    match failure.kind() {
        MatrixFailureKind::InvalidConfiguration => "matrix.invalid_configuration",
        MatrixFailureKind::Unauthenticated => "matrix.unauthenticated",
        MatrixFailureKind::AuthenticationRejected => "matrix.authentication_rejected",
        MatrixFailureKind::Forbidden => "matrix.forbidden",
        MatrixFailureKind::NotFound => "matrix.not_found",
        MatrixFailureKind::Conflict => "matrix.conflict",
        MatrixFailureKind::RateLimited => "matrix.rate_limited",
        MatrixFailureKind::Timeout => "matrix.timeout",
        MatrixFailureKind::DependencyUnavailable => "matrix.unavailable",
        MatrixFailureKind::InvalidResponse => "matrix.invalid_response",
        MatrixFailureKind::UnknownCommit => "matrix.unknown_commit",
        MatrixFailureKind::StaleSyncToken => "matrix.stale_sync_token",
        MatrixFailureKind::UnsupportedVersion => "matrix.unsupported_version",
    }
}
