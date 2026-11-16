//! 项目房间的认领在后台核对（`specs/redesign/project-rooms.md` 的“认领、核对、绑定”，3P-2b）：每分钟
//! 一轮，作废 7 天还没认上的，再核对该核对的（头 24 小时每 5 分钟一次，之后每小时一次）。与 HTTP 服务
//! 共同存活、共同关闭，启动时做一轮。多个控制面同时跑时，同一个认领靠“开始核对”那一下的时间戳分开，
//! 不会两边都去读 GitHub。

use std::{sync::Arc, time::Duration};

use agent_room_application::project_rooms::ProjectRoomClaimChecks;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

const INTERVAL: Duration = Duration::from_mins(1);

pub(crate) struct ProjectRoomCheckWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ProjectRoomCheckWorker {
    pub(crate) fn start(checks: Arc<dyn ProjectRoomClaimChecks>) -> Self {
        Self::start_every(checks, INTERVAL)
    }

    fn start_every(checks: Arc<dyn ProjectRoomClaimChecks>, period: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(checks, period, stop_requested));
        Self {
            stop: Some(stop),
            task,
        }
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Err(error) = self.task.await {
            tracing::error!(
                code = "project_room.claim_checks_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "项目房间认领的后台核对异常结束"
            );
        }
    }
}

async fn run_worker(
    checks: Arc<dyn ProjectRoomClaimChecks>,
    period: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticks = interval(period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => match checks.check_due().await {
                Ok(round) if round.checked == 0 && round.expired == 0 => {}
                Ok(round) => tracing::info!(
                    expired = round.expired,
                    checked = round.checked,
                    bound = round.bound,
                    pending = round.pending,
                    dropped = round.dropped,
                    deferred = round.deferred,
                    "做了一轮项目房间认领的核对"
                ),
                Err(error) => tracing::warn!(
                    operation = error.operation(),
                    kind = ?error.kind(),
                    "这一轮没读到要核对的认领，一分钟后再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("项目房间认领的后台核对已停止");
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use agent_room_application::{
        persistence::RepositoryError,
        ports::PortFuture,
        project_rooms::{ClaimRound, ProjectRoomClaimChecks},
    };
    use tokio::sync::Notify;

    use super::ProjectRoomCheckWorker;

    #[derive(Default)]
    struct RecordingChecks {
        calls: AtomicUsize,
        called: Notify,
    }

    impl ProjectRoomClaimChecks for RecordingChecks {
        fn check_due(&self) -> PortFuture<'_, Result<ClaimRound, RepositoryError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(ClaimRound {
                    checked: 1,
                    pending: 1,
                    ..ClaimRound::default()
                })
            })
        }
    }

    #[tokio::test]
    async fn 启动就做一轮_之后一轮一轮地做_关掉以后不再跑() {
        let checks = Arc::new(RecordingChecks::default());
        let called = checks.called.notified();
        let worker = ProjectRoomCheckWorker::start_every(checks.clone(), Duration::from_mins(1));
        tokio::time::timeout(Duration::from_secs(1), called)
            .await
            .expect("启动就做一轮");
        assert_eq!(checks.calls.load(Ordering::SeqCst), 1);
        worker.shutdown().await;

        let checks = Arc::new(RecordingChecks::default());
        let worker = ProjectRoomCheckWorker::start_every(checks.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
        worker.shutdown().await;
        let calls_after_shutdown = checks.calls.load(Ordering::SeqCst);
        assert!(calls_after_shutdown >= 2, "{calls_after_shutdown}");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(checks.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
