use std::sync::{Arc, Mutex};

use agent_room_application::{
    account_lifecycle::{
        AccountDeletionWorker, AccountDeletionWorkerDependencies, AccountDeletionWorkerOutcome,
        AccountLifecycleDependencies, AccountLifecycleFailureKind, AccountLifecycleService,
        AccountLifecycleUseCases, InspectAccountDeletion, ReplayAccountDeletion,
        RequestAccountDeletion,
    },
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        AccountDeletionClaim, AccountDeletionReceiptIssuer, AccountDeletionRepository,
        AccountDeletionRequest, AccountDeletionRequestOutcome, AccountDeletionStage,
        AccountDeletionStatus, AccountExportSnapshot, Clock, MatrixAccountLifecycleGateway,
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixUserId, OidcFailure,
        OidcFailureKind, OidcResult, PortFuture, SecretDigest, SecretGenerationFailure,
        SecretValue, SignInAccount, SignInAccountRemoval,
    },
};
use agent_room_domain::{
    ids::{AccountDeletionJobId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};
use serde_json::json;
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;
const RECEIPT: &str = "account-deletion-receipt-with-at-least-256-bits-of-test-entropy";

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct FixedSecrets;

impl AccountDeletionReceiptIssuer for FixedSecrets {
    fn issue(&self, _job_id: AccountDeletionJobId) -> Result<SecretValue, SecretGenerationFailure> {
        SecretValue::new(RECEIPT).map_err(|_| SecretGenerationFailure::EntropyUnavailable)
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut bytes = [0_u8; 32];
        for (index, byte) in value.bytes().enumerate() {
            let slot = index % bytes.len();
            bytes[slot] = bytes[slot].wrapping_mul(31).wrapping_add(byte);
        }
        SecretDigest::from_array(bytes)
    }
}

struct FakeRepository {
    request_status: AccountDeletionStatus,
    request_is_existing: bool,
    requested: Mutex<Vec<AccountDeletionRequest>>,
    found: Mutex<Option<AccountDeletionStatus>>,
    claim: Mutex<Option<AccountDeletionClaim>>,
    retries: Mutex<Vec<(String, UtcMillis)>>,
    federated_records: Mutex<u16>,
    finalizations: Mutex<u16>,
}

impl FakeRepository {
    fn new(status: AccountDeletionStatus) -> Self {
        Self {
            request_status: status,
            request_is_existing: false,
            requested: Mutex::new(Vec::new()),
            found: Mutex::new(None),
            claim: Mutex::new(None),
            retries: Mutex::new(Vec::new()),
            federated_records: Mutex::new(0),
            finalizations: Mutex::new(0),
        }
    }

    fn existing(status: AccountDeletionStatus) -> Self {
        Self {
            request_is_existing: true,
            ..Self::new(status)
        }
    }
}

impl AccountDeletionRepository for FakeRepository {
    fn export(
        &self,
        _principal_id: PrincipalId,
        generated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AccountExportSnapshot>>> {
        Box::pin(async move {
            Ok(Some(AccountExportSnapshot {
                schema_version: 1,
                generated_at,
                data: json!({"principal": {"status": "active"}}),
            }))
        })
    }

    fn request<'a>(
        &'a self,
        request: &'a AccountDeletionRequest,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionRequestOutcome>> {
        Box::pin(async move {
            self.requested
                .lock()
                .expect("测试锁不得中毒")
                .push(request.clone());
            let status = self.request_status.clone();
            Ok(if self.request_is_existing {
                AccountDeletionRequestOutcome::Existing(status)
            } else {
                AccountDeletionRequestOutcome::Created(status)
            })
        })
    }

    fn find_by_receipt<'a>(
        &'a self,
        _receipt_digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<AccountDeletionStatus>>> {
        Box::pin(async move { Ok(self.found.lock().expect("测试锁不得中毒").clone()) })
    }

    fn claim_due(
        &self,
        _now: UtcMillis,
        _lease_expires_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AccountDeletionClaim>>> {
        Box::pin(async move { Ok(self.claim.lock().expect("测试锁不得中毒").take()) })
    }

    fn record_federated_deactivation<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        _completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionClaim>> {
        Box::pin(async move {
            *self.federated_records.lock().expect("测试锁不得中毒") += 1;
            let mut next = claim.clone();
            next.stage = AccountDeletionStage::LocalErasure;
            next.version += 1;
            Ok(next)
        })
    }

    fn schedule_retry<'a>(
        &'a self,
        _claim: &'a AccountDeletionClaim,
        failure_code: &'a str,
        retry_at: UtcMillis,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.retries
                .lock()
                .expect("测试锁不得中毒")
                .push((failure_code.to_owned(), retry_at));
            Ok(())
        })
    }

    fn finalize_local<'a>(
        &'a self,
        claim: &'a AccountDeletionClaim,
        completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<AccountDeletionStatus>> {
        Box::pin(async move {
            *self.finalizations.lock().expect("测试锁不得中毒") += 1;
            Ok(AccountDeletionStatus {
                job_id: claim.job_id,
                stage: AccountDeletionStage::Completed,
                attempt_count: claim.attempt_count,
                requested_at: time(NOW),
                updated_at: completed_at,
                retry_at: None,
                completed_at: Some(completed_at),
                failure_code: None,
            })
        })
    }
}

/// 两个外部账户按什么顺序关掉。
type Steps = Arc<Mutex<Vec<&'static str>>>;

struct FakeMatrix {
    outcome: MatrixResult<()>,
    calls: Mutex<Vec<MatrixUserId>>,
    steps: Steps,
}

impl FakeMatrix {
    fn new(outcome: MatrixResult<()>, steps: &Steps) -> Self {
        Self {
            outcome,
            calls: Mutex::new(Vec::new()),
            steps: steps.clone(),
        }
    }
}

impl MatrixAccountLifecycleGateway for FakeMatrix {
    fn deactivate_and_erase<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("测试锁不得中毒")
                .push(user_id.clone());
            self.steps.lock().expect("测试锁不得中毒").push("matrix");
            self.outcome
        })
    }
}

struct FakeSignIn {
    outcome: OidcResult<()>,
    calls: Mutex<Vec<SignInAccount>>,
    steps: Steps,
}

impl FakeSignIn {
    fn new(outcome: OidcResult<()>, steps: &Steps) -> Self {
        Self {
            outcome,
            calls: Mutex::new(Vec::new()),
            steps: steps.clone(),
        }
    }
}

impl SignInAccountRemoval for FakeSignIn {
    fn remove<'a>(&'a self, account: &'a SignInAccount) -> PortFuture<'a, OidcResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("测试锁不得中毒")
                .push(account.clone());
            self.steps.lock().expect("测试锁不得中毒").push("sign_in");
            self.outcome
        })
    }
}

#[tokio::test]
async fn 删除请求必须近期认证并明确确认联邦残留() {
    let status = queued_status();
    let repository = Arc::new(FakeRepository::new(status));
    let service = service(repository.clone());

    for (recent, confirmation, acknowledged, expected) in [
        (
            false,
            "DELETE",
            true,
            AccountLifecycleFailureKind::Forbidden,
        ),
        (
            true,
            "delete",
            true,
            AccountLifecycleFailureKind::InvalidRequest,
        ),
        (
            true,
            "DELETE",
            false,
            AccountLifecycleFailureKind::InvalidRequest,
        ),
    ] {
        let failure = service
            .request_deletion(RequestAccountDeletion {
                actor: actor(recent),
                job_id: AccountDeletionJobId::from_uuid(Uuid::now_v7()),
                confirmation: confirmation.to_owned(),
                federation_residual_acknowledged: acknowledged,
            })
            .await
            .expect_err("不满足删除门禁时必须拒绝");
        assert_eq!(failure.kind(), expected);
    }
    assert!(
        repository
            .requested
            .lock()
            .expect("测试锁不得中毒")
            .is_empty()
    );
}

#[tokio::test]
async fn 删除回执只返回一次且仓储仅接收摘要() {
    let status = queued_status();
    let repository = Arc::new(FakeRepository::new(status.clone()));
    *repository.found.lock().expect("测试锁不得中毒") = Some(status.clone());
    let service = service(repository.clone());
    let started = service
        .request_deletion(RequestAccountDeletion {
            actor: actor(true),
            job_id: status.job_id,
            confirmation: "DELETE".to_owned(),
            federation_residual_acknowledged: true,
        })
        .await
        .expect("有效删除请求应入队");

    assert_eq!(started.receipt.expose(), RECEIPT);
    {
        let request = repository.requested.lock().expect("测试锁不得中毒");
        assert_eq!(request.len(), 1);
        assert_eq!(request[0].receipt_digest, FixedSecrets.digest(RECEIPT));
    }
    let inspected = service
        .inspect_deletion(InspectAccountDeletion {
            receipt: started.receipt,
        })
        .await
        .expect("回执应能读取进度");
    assert_eq!(inspected, status);
}

#[tokio::test]
async fn 相同幂等任务在响应丢失后稳定重发同一回执() {
    let status = queued_status();
    let repository = Arc::new(FakeRepository::existing(status.clone()));
    *repository.found.lock().expect("测试锁不得中毒") = Some(status.clone());
    let service = service(repository);
    let replayed = service
        .request_deletion(RequestAccountDeletion {
            actor: actor(true),
            job_id: status.job_id,
            confirmation: "DELETE".to_owned(),
            federation_residual_acknowledged: true,
        })
        .await
        .expect("同一幂等任务必须能安全重放");
    assert_eq!(replayed.receipt.expose(), RECEIPT);
    assert_eq!(replayed.status, status);

    let unauthenticated_replay = service
        .replay_deletion(ReplayAccountDeletion {
            job_id: status.job_id,
            confirmation: "DELETE".to_owned(),
            federation_residual_acknowledged: true,
        })
        .await
        .expect("会话撤销后仍应能重放同一任务");
    assert_eq!(unauthenticated_replay.receipt.expose(), RECEIPT);
    assert_eq!(unauthenticated_replay.status, status);
}

#[tokio::test]
async fn matrix_失败时指数退避且不得提前擦除本地数据() {
    let status = queued_status();
    let repository = Arc::new(FakeRepository::new(status.clone()));
    *repository.claim.lock().expect("测试锁不得中毒") = Some(claim(
        status.job_id,
        AccountDeletionStage::FederatedDeactivation,
        2,
    ));
    let steps = Steps::default();
    let matrix = Arc::new(FakeMatrix::new(
        Err(MatrixFailure::new(
            MatrixOperation::DeactivateAccount,
            MatrixFailureKind::Timeout,
        )),
        &steps,
    ));
    let sign_in = Arc::new(FakeSignIn::new(Ok(()), &steps));
    let worker = worker(repository.clone(), matrix, sign_in);

    assert_eq!(
        worker.run_once().await.expect("退避调度应成功"),
        AccountDeletionWorkerOutcome::Retrying(status.job_id)
    );
    assert_eq!(
        repository
            .retries
            .lock()
            .expect("测试锁不得中毒")
            .as_slice(),
        &[("matrix.timeout".to_owned(), time(NOW + 10_000))]
    );
    assert_eq!(*repository.finalizations.lock().expect("测试锁不得中毒"), 0);
}

#[tokio::test]
async fn 登录服务暂时不可用时退避重试_聊天账户和本地数据都不动() {
    let status = queued_status();
    let repository = Arc::new(FakeRepository::new(status.clone()));
    *repository.claim.lock().expect("测试锁不得中毒") = Some(claim(
        status.job_id,
        AccountDeletionStage::FederatedDeactivation,
        1,
    ));
    let steps = Steps::default();
    let matrix = Arc::new(FakeMatrix::new(Ok(()), &steps));
    let sign_in = Arc::new(FakeSignIn::new(
        Err(OidcFailure::new(OidcFailureKind::DependencyUnavailable)),
        &steps,
    ));
    let worker = worker(repository.clone(), matrix.clone(), sign_in);

    assert_eq!(
        worker.run_once().await.expect("退避调度应成功"),
        AccountDeletionWorkerOutcome::Retrying(status.job_id)
    );
    assert_eq!(
        repository
            .retries
            .lock()
            .expect("测试锁不得中毒")
            .as_slice(),
        &[("identity.unavailable".to_owned(), time(NOW + 5_000))]
    );
    assert!(matrix.calls.lock().expect("测试锁不得中毒").is_empty());
    assert_eq!(
        *repository.federated_records.lock().expect("测试锁不得中毒"),
        0
    );
    assert_eq!(*repository.finalizations.lock().expect("测试锁不得中毒"), 0);
}

#[tokio::test]
async fn 删除工作流先删登录账户_再停用_matrix_最后本地匿名化() {
    let status = queued_status();
    let repository = Arc::new(FakeRepository::new(status.clone()));
    *repository.claim.lock().expect("测试锁不得中毒") = Some(claim(
        status.job_id,
        AccountDeletionStage::FederatedDeactivation,
        1,
    ));
    let steps = Steps::default();
    let matrix = Arc::new(FakeMatrix::new(Ok(()), &steps));
    let sign_in = Arc::new(FakeSignIn::new(Ok(()), &steps));
    let worker = worker(repository.clone(), matrix.clone(), sign_in.clone());

    assert_eq!(
        worker.run_once().await.expect("完整删除应成功"),
        AccountDeletionWorkerOutcome::Completed(status.job_id)
    );
    assert_eq!(matrix.calls.lock().expect("测试锁不得中毒").len(), 1);
    assert_eq!(
        sign_in.calls.lock().expect("测试锁不得中毒").as_slice(),
        &[sign_in_account()]
    );
    // 先删登录账户：之后就登录不了，再停用聊天账户。
    assert_eq!(
        steps.lock().expect("测试锁不得中毒").as_slice(),
        &["sign_in", "matrix"]
    );
    assert_eq!(
        *repository.federated_records.lock().expect("测试锁不得中毒"),
        1
    );
    assert_eq!(*repository.finalizations.lock().expect("测试锁不得中毒"), 1);
}

fn service(repository: Arc<FakeRepository>) -> AccountLifecycleService {
    AccountLifecycleService::new(AccountLifecycleDependencies {
        repository,
        receipts: Arc::new(FixedSecrets),
        clock: Arc::new(FixedClock),
    })
}

fn worker(
    repository: Arc<FakeRepository>,
    matrix: Arc<FakeMatrix>,
    sign_in: Arc<FakeSignIn>,
) -> AccountDeletionWorker {
    AccountDeletionWorker::new(AccountDeletionWorkerDependencies {
        repository,
        matrix,
        sign_in,
        clock: Arc::new(FixedClock),
        lease_duration: duration(30_000),
        initial_retry_delay: duration(5_000),
        maximum_retry_delay: duration(60_000),
    })
}

fn actor(recently_authenticated: bool) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: "@alice:matrix.agent-room.localhost".to_owned(),
        display_name: "Alice".to_owned(),
        locale: "en".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated,
    }
}

fn queued_status() -> AccountDeletionStatus {
    AccountDeletionStatus {
        job_id: AccountDeletionJobId::from_uuid(Uuid::now_v7()),
        stage: AccountDeletionStage::Queued,
        attempt_count: 0,
        requested_at: time(NOW),
        updated_at: time(NOW),
        retry_at: None,
        completed_at: None,
        failure_code: None,
    }
}

fn claim(
    job_id: AccountDeletionJobId,
    stage: AccountDeletionStage,
    attempt_count: u16,
) -> AccountDeletionClaim {
    AccountDeletionClaim {
        job_id,
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: MatrixUserId::new("@alice:matrix.agent-room.localhost".to_owned())
            .expect("测试 MXID 有效"),
        sign_in_account: sign_in_account(),
        stage,
        attempt_count,
        version: 1,
    }
}

fn sign_in_account() -> SignInAccount {
    SignInAccount {
        issuer: "https://identity.agent-room.localhost/realms/agent-room".to_owned(),
        subject: "0b9a1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d".to_owned(),
    }
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}

fn duration(value: u64) -> DurationMillis {
    DurationMillis::new(value).expect("测试时长有效")
}
