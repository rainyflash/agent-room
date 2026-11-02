//! 申请进公开房间的定时清理（`specs/public-rooms/design.md`）：不公开了、归档了的房间里在等的作废，
//! 处理完 30 天的删掉。与 HTTP 服务共同存活、共同关闭，启动时清一次、之后每小时一次。

use std::{sync::Arc, time::Duration};

use agent_room_application::private_rooms::JoinRequestCleanupUseCases;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

const INTERVAL: Duration = Duration::from_hours(1);

pub(crate) struct JoinRequestCleanupWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl JoinRequestCleanupWorker {
    pub(crate) fn start(cleanup: Arc<dyn JoinRequestCleanupUseCases>) -> Self {
        Self::start_every(cleanup, INTERVAL)
    }

    fn start_every(cleanup: Arc<dyn JoinRequestCleanupUseCases>, period: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(cleanup, period, stop_requested));
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
                code = "private_room.join_request_cleanup_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "申请进公开房间的定时清理异常结束"
            );
        }
    }
}

async fn run_worker(
    cleanup: Arc<dyn JoinRequestCleanupUseCases>,
    period: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticks = interval(period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => match cleanup.sweep().await {
                Ok(sweep) if sweep.voided == 0 && sweep.deleted == 0 => {}
                Ok(sweep) => tracing::info!(
                    voided = sweep.voided,
                    deleted = sweep.deleted,
                    "清理了一轮申请进公开房间的记录"
                ),
                Err(failure) => tracing::warn!(
                    operation = failure.operation(),
                    kind = ?failure.kind(),
                    "这次没清理成申请进公开房间的记录，一小时后再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("申请进公开房间的定时清理已停止");
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
        ports::{JoinRequestSweep, PortFuture},
        private_rooms::{JoinRequestCleanupUseCases, PublicJoinResult},
    };
    use tokio::sync::Notify;

    use super::JoinRequestCleanupWorker;

    #[derive(Default)]
    struct RecordingCleanup {
        calls: AtomicUsize,
        called: Notify,
    }

    impl JoinRequestCleanupUseCases for RecordingCleanup {
        fn sweep(&self) -> PortFuture<'_, PublicJoinResult<JoinRequestSweep>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(JoinRequestSweep {
                    voided: 1,
                    deleted: 2,
                })
            })
        }
    }

    #[tokio::test]
    async fn 启动就清一次_之后一轮一轮地清_关掉以后不再跑() {
        let cleanup = Arc::new(RecordingCleanup::default());
        let called = cleanup.called.notified();
        let worker = JoinRequestCleanupWorker::start_every(cleanup.clone(), Duration::from_mins(1));
        tokio::time::timeout(Duration::from_secs(1), called)
            .await
            .expect("启动就清一次");
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), 1);
        worker.shutdown().await;

        let cleanup = Arc::new(RecordingCleanup::default());
        let worker =
            JoinRequestCleanupWorker::start_every(cleanup.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
        worker.shutdown().await;
        let calls_after_shutdown = cleanup.calls.load(Ordering::SeqCst);
        assert!(calls_after_shutdown >= 2, "{calls_after_shutdown}");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
