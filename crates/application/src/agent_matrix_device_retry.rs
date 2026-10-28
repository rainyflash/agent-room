//! 撤销以后 Matrix 设备没注销成的，后台再试（`specs/outbox/design.md` 第 4 步）。
//!
//! 断开连接、撤销电脑都在请求里先注销这个连接的 Matrix 设备，那一刻聊天服务器不通就没注销成。以前要人
//! 再点一次撤销；桌面端换账号、移出这台电脑的那条路上，这台电脑的凭据已经删了，没人能再点。控制面的
//! 定时任务每 30 秒找一次撤销了、还没注销成的连接再试，注销成了记下 `matrix_device_revoked_at`。
//!
//! 不另建表：要不要再试只看 `agent_instance` 自己。注销是幂等的（设备已经没了也算注销成），多个控制面
//! 副本同时试也没事。退避只记在内存里，重启以后从头来。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_domain::{ids::AgentInstanceId, time::UtcMillis};

use crate::{
    matrix_device_cleanup::{MatrixDeviceCleanupFailure, revoke_agent_matrix_device},
    moderation::{homeserver_unavailable, matrix_failure_code},
    persistence::RepositoryError,
    ports::{
        AgentInstanceMatrixCleanupStore, Clock, MatrixAgentDeviceSessionRevoker, MatrixFailure,
        MatrixOperation, PendingAgentMatrixDeviceRevocation, PendingAgentMatrixDeviceStore,
        PortFuture,
    },
};

/// 一轮最多读这么多条。平时一条也没有。
const PENDING_PER_ROUND: u16 = 200;
/// 一轮最多试这么多条，再多的留给下一轮。
const ATTEMPTS_PER_ROUND: usize = 20;
/// 撤销的那次请求里还在注销，这么久以内撤销的先不碰。
const REQUEST_GRACE_MILLIS: i64 = 30 * 1000;
/// 没注销成时第一次隔多久再试，之后每次翻倍。
const RETRY_INITIAL_MILLIS: i64 = 30 * 1000;
/// 最长隔多久再试。不设放弃：放弃了这台设备就一直登着。
const RETRY_MAXIMUM_MILLIS: i64 = 15 * 60 * 1000;
const INVALID_IDENTITY_CODE: &str = "agent_instance.invalid_matrix_identity";
const STATE_UNAVAILABLE_CODE: &str = "agent_instance.cleanup_state_unavailable";

/// 撤销以后没注销成的 Matrix 设备再试。
pub trait AgentMatrixDeviceRetryUseCases: Send + Sync {
    /// 定时任务的一轮：试到期的。
    fn retry_due(&self) -> PortFuture<'_, Result<MatrixDeviceRetryRound, RepositoryError>>;
}

/// 一轮做成什么样。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatrixDeviceRetryRound {
    /// 这一轮注销成了几台。
    pub revoked: usize,
    /// 没注销成、往后退避再试的。
    pub retrying: Vec<MatrixDeviceRetry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixDeviceRetry {
    pub instance_id: AgentInstanceId,
    /// 和治理动作失败时记的错误码是一套，比如 `matrix.unavailable`。
    pub failure_code: &'static str,
    /// 这个进程里第一次没注销成：只有这一次值得告警，之后的重试只记调试日志。
    pub first_failure: bool,
}

pub struct AgentMatrixDeviceRetryDependencies {
    pub pending: Arc<dyn PendingAgentMatrixDeviceStore>,
    pub matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    pub cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    pub clock: Arc<dyn Clock>,
}

pub struct AgentMatrixDeviceRetryService {
    pending: Arc<dyn PendingAgentMatrixDeviceStore>,
    matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    clock: Arc<dyn Clock>,
    backoff: Mutex<HashMap<AgentInstanceId, Backoff>>,
}

#[derive(Debug, Clone, Copy)]
struct Backoff {
    failures: u32,
    next_attempt_at: UtcMillis,
}

impl AgentMatrixDeviceRetryService {
    pub fn new(dependencies: AgentMatrixDeviceRetryDependencies) -> Self {
        Self {
            pending: dependencies.pending,
            matrix: dependencies.matrix,
            cleanup: dependencies.cleanup,
            clock: dependencies.clock,
            backoff: Mutex::new(HashMap::new()),
        }
    }

    async fn round(&self) -> Result<MatrixDeviceRetryRound, RepositoryError> {
        let now = self.clock.now();
        let revoked_before =
            UtcMillis::new(now.value().saturating_sub(REQUEST_GRACE_MILLIS)).unwrap_or(now);
        let pending = self
            .pending
            .list_pending_matrix_devices(revoked_before, PENDING_PER_ROUND)
            .await?;
        let due = self.due(pending, now);
        let mut round = MatrixDeviceRetryRound::default();
        for target in due {
            let result = revoke_agent_matrix_device(
                self.matrix.as_ref(),
                self.cleanup.as_ref(),
                target.instance_id,
                &target.matrix_user_id,
                &target.matrix_device_id,
                now,
            )
            .await;
            match result {
                Ok(()) => {
                    self.backoff_table().remove(&target.instance_id);
                    round.revoked += 1;
                }
                Err(failure) => {
                    let (failure_code, homeserver_down) = describe(failure);
                    let first_failure = self.record_failure(target.instance_id, now);
                    round.retrying.push(MatrixDeviceRetry {
                        instance_id: target.instance_id,
                        failure_code,
                        first_failure,
                    });
                    // 整台聊天服务器这会儿用不了，别的也不用再试了。
                    if homeserver_down {
                        break;
                    }
                }
            }
        }
        Ok(round)
    }

    /// 到了该试的时候的那些。读到的不是全部时，不在里面的照旧记着退避。
    fn due(
        &self,
        pending: Vec<PendingAgentMatrixDeviceRevocation>,
        now: UtcMillis,
    ) -> Vec<PendingAgentMatrixDeviceRevocation> {
        let mut backoff = self.backoff_table();
        if pending.len() < usize::from(PENDING_PER_ROUND) {
            // 别处注销成了（比如人又点了一次撤销）的不用再记。
            backoff.retain(|instance_id, _| {
                pending
                    .iter()
                    .any(|target| target.instance_id == *instance_id)
            });
        }
        pending
            .into_iter()
            .filter(|target| {
                backoff
                    .get(&target.instance_id)
                    .is_none_or(|entry| entry.next_attempt_at <= now)
            })
            .take(ATTEMPTS_PER_ROUND)
            .collect()
    }

    /// 记下又一次没注销成；这个进程里第一次没注销成时返回 `true`。
    fn record_failure(&self, instance_id: AgentInstanceId, now: UtcMillis) -> bool {
        let mut backoff = self.backoff_table();
        let failures = backoff
            .get(&instance_id)
            .map_or(1, |entry| entry.failures.saturating_add(1));
        let delay = retry_delay(failures);
        let next_attempt_at = UtcMillis::new(now.value().saturating_add(delay)).unwrap_or(now);
        backoff.insert(
            instance_id,
            Backoff {
                failures,
                next_attempt_at,
            },
        );
        failures == 1
    }

    fn backoff_table(&self) -> std::sync::MutexGuard<'_, HashMap<AgentInstanceId, Backoff>> {
        self.backoff.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl AgentMatrixDeviceRetryUseCases for AgentMatrixDeviceRetryService {
    fn retry_due(&self) -> PortFuture<'_, Result<MatrixDeviceRetryRound, RepositoryError>> {
        Box::pin(self.round())
    }
}

/// 第 n 次没注销成以后隔多久再试：30 秒起翻倍，最多 15 分钟。
fn retry_delay(failures: u32) -> i64 {
    let doublings = failures.saturating_sub(1).min(16);
    RETRY_INITIAL_MILLIS
        .saturating_mul(1_i64 << doublings)
        .min(RETRY_MAXIMUM_MILLIS)
}

/// 没注销成的原因，和这会儿是不是整台聊天服务器（或者库）都用不了。
fn describe(failure: MatrixDeviceCleanupFailure) -> (&'static str, bool) {
    match failure {
        MatrixDeviceCleanupFailure::Matrix(kind) => {
            let failure = MatrixFailure::new(MatrixOperation::RevokeAgentDeviceSession, kind);
            (
                matrix_failure_code(failure),
                homeserver_unavailable(failure),
            )
        }
        MatrixDeviceCleanupFailure::InvalidStoredIdentity => (INVALID_IDENTITY_CODE, false),
        // 注销成了、记不下来：库这会儿用不了，下一轮再试（再注销一次也没事）。
        MatrixDeviceCleanupFailure::StatePersistenceUnavailable => (STATE_UNAVAILABLE_CODE, true),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use agent_room_domain::{ids::AgentInstanceId, time::UtcMillis};
    use uuid::Uuid;

    use super::{
        AgentMatrixDeviceRetryDependencies, AgentMatrixDeviceRetryService,
        AgentMatrixDeviceRetryUseCases, MatrixDeviceRetry,
    };
    use crate::{
        persistence::RepositoryResult,
        ports::{
            AgentInstanceMatrixCleanupStore, Clock, MatrixAgentDeviceSessionRevoker,
            MatrixAgentDeviceSessionTarget, MatrixFailure, MatrixFailureKind, MatrixOperation,
            MatrixResult, PendingAgentMatrixDeviceRevocation, PendingAgentMatrixDeviceStore,
            PortFuture,
        },
    };

    const START: i64 = 1_700_000_000_000;

    #[derive(Default)]
    struct Store {
        pending: Mutex<Vec<PendingAgentMatrixDeviceRevocation>>,
        asked_before: Mutex<Vec<UtcMillis>>,
    }

    impl PendingAgentMatrixDeviceStore for Store {
        fn list_pending_matrix_devices(
            &self,
            revoked_before: UtcMillis,
            _limit: u16,
        ) -> PortFuture<'_, RepositoryResult<Vec<PendingAgentMatrixDeviceRevocation>>> {
            self.asked_before
                .lock()
                .expect("测试锁不得中毒")
                .push(revoked_before);
            let pending = self.pending.lock().expect("测试锁不得中毒").clone();
            Box::pin(async move { Ok(pending) })
        }
    }

    impl AgentInstanceMatrixCleanupStore for Store {
        fn mark_matrix_device_revoked(
            &self,
            instance_id: AgentInstanceId,
            _revoked_at: UtcMillis,
        ) -> PortFuture<'_, RepositoryResult<()>> {
            self.pending
                .lock()
                .expect("测试锁不得中毒")
                .retain(|target| target.instance_id != instance_id);
            Box::pin(async { Ok(()) })
        }
    }

    /// 按顺序给出每次注销的结果，给完以后都算注销成。
    #[derive(Default)]
    struct Matrix {
        failures: Mutex<VecDeque<MatrixFailureKind>>,
        attempts: Mutex<Vec<String>>,
    }

    impl MatrixAgentDeviceSessionRevoker for Matrix {
        fn revoke_device_session<'a>(
            &'a self,
            target: &'a MatrixAgentDeviceSessionTarget,
        ) -> PortFuture<'a, MatrixResult<()>> {
            self.attempts
                .lock()
                .expect("测试锁不得中毒")
                .push(target.device_id().as_str().to_owned());
            let failure = self.failures.lock().expect("测试锁不得中毒").pop_front();
            Box::pin(async move {
                failure.map_or(Ok(()), |kind| {
                    Err(MatrixFailure::new(
                        MatrixOperation::RevokeAgentDeviceSession,
                        kind,
                    ))
                })
            })
        }
    }

    struct SteppingClock(Mutex<i64>);

    impl SteppingClock {
        fn advance(&self, milliseconds: i64) {
            *self.0.lock().expect("测试锁不得中毒") += milliseconds;
        }
    }

    impl Clock for SteppingClock {
        fn now(&self) -> UtcMillis {
            UtcMillis::new(*self.0.lock().expect("测试锁不得中毒")).expect("测试时间有效")
        }
    }

    struct Fixture {
        store: Arc<Store>,
        matrix: Arc<Matrix>,
        clock: Arc<SteppingClock>,
        service: AgentMatrixDeviceRetryService,
    }

    fn fixture(pending: Vec<PendingAgentMatrixDeviceRevocation>) -> Fixture {
        let store = Arc::new(Store::default());
        *store.pending.lock().expect("测试锁不得中毒") = pending;
        let matrix = Arc::new(Matrix::default());
        let clock = Arc::new(SteppingClock(Mutex::new(START)));
        let service = AgentMatrixDeviceRetryService::new(AgentMatrixDeviceRetryDependencies {
            pending: store.clone(),
            matrix: matrix.clone(),
            cleanup: store.clone(),
            clock: clock.clone(),
        });
        Fixture {
            store,
            matrix,
            clock,
            service,
        }
    }

    fn pending(device: &str) -> PendingAgentMatrixDeviceRevocation {
        PendingAgentMatrixDeviceRevocation {
            instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
            matrix_user_id: "@_agent_0198b60177a17bb883eba8fe68c97e42:matrix.example.test"
                .to_owned(),
            matrix_device_id: device.to_owned(),
        }
    }

    fn fail_next(fixture: &Fixture, kinds: &[MatrixFailureKind]) {
        fixture
            .matrix
            .failures
            .lock()
            .expect("测试锁不得中毒")
            .extend(kinds.iter().copied());
    }

    fn attempts(fixture: &Fixture) -> Vec<String> {
        fixture
            .matrix
            .attempts
            .lock()
            .expect("测试锁不得中毒")
            .clone()
    }

    #[tokio::test]
    async fn 注销成的记下_刚撤销的留给撤销请求自己做() {
        let fixture = fixture(vec![pending("AR_DEVICE_A")]);

        let round = fixture.service.retry_due().await.expect("一轮应做完");

        assert_eq!(round.revoked, 1);
        assert!(round.retrying.is_empty());
        assert!(
            fixture
                .store
                .pending
                .lock()
                .expect("测试锁不得中毒")
                .is_empty()
        );
        assert_eq!(
            *fixture.store.asked_before.lock().expect("测试锁不得中毒"),
            [UtcMillis::new(START - 30_000).expect("测试时间有效")]
        );
    }

    #[tokio::test]
    async fn 没注销成的往后退避_第一次才算告警() {
        let target = pending("AR_DEVICE_A");
        let instance_id = target.instance_id;
        let fixture = fixture(vec![target]);
        fail_next(
            &fixture,
            &[MatrixFailureKind::Forbidden, MatrixFailureKind::Forbidden],
        );

        let first = fixture.service.retry_due().await.expect("第一轮应做完");
        assert_eq!(
            first.retrying,
            [MatrixDeviceRetry {
                instance_id,
                failure_code: "matrix.forbidden",
                first_failure: true,
            }]
        );
        fixture.clock.advance(29_000);
        let waiting = fixture
            .service
            .retry_due()
            .await
            .expect("退避中的一轮应做完");
        assert!(waiting.retrying.is_empty());
        assert_eq!(attempts(&fixture).len(), 1);

        fixture.clock.advance(1_000);
        let second = fixture.service.retry_due().await.expect("第二轮应做完");
        assert!(!second.retrying[0].first_failure);
        fixture.clock.advance(59_000);
        assert_eq!(
            fixture
                .service
                .retry_due()
                .await
                .expect("翻倍的退避里不试")
                .revoked,
            0
        );
        fixture.clock.advance(1_000);
        let third = fixture.service.retry_due().await.expect("第三轮应做完");
        assert_eq!(third.revoked, 1);
        assert_eq!(attempts(&fixture).len(), 3);
    }

    #[tokio::test]
    async fn 聊天服务器整个不通时这一轮先停() {
        let fixture = fixture(vec![pending("AR_DEVICE_A"), pending("AR_DEVICE_B")]);
        fail_next(&fixture, &[MatrixFailureKind::DependencyUnavailable]);

        let round = fixture.service.retry_due().await.expect("一轮应做完");

        assert_eq!(round.retrying.len(), 1);
        assert_eq!(round.retrying[0].failure_code, "matrix.unavailable");
        assert_eq!(attempts(&fixture), ["AR_DEVICE_A"]);
        let next = fixture.service.retry_due().await.expect("下一轮应做完");
        assert_eq!(next.revoked, 1);
        assert_eq!(attempts(&fixture), ["AR_DEVICE_A", "AR_DEVICE_B"]);
    }
}
