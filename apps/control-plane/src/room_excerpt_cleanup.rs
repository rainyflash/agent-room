//! 分享的一段对话的定时清理（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7a）：第一句过了房间
//! 保留期的、房间没在放实况的段删掉。关实况、关房间、删账户时已经在同一个请求里删了，这一轮是兜底。
//! 与 HTTP 服务共同存活、共同关闭，启动时清一次、之后每天一次。

use std::{sync::Arc, time::Duration};

use agent_room_application::room_excerpts::RoomExcerptCleanupUseCases;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

const INTERVAL: Duration = Duration::from_hours(24);

pub(crate) struct RoomExcerptCleanupWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RoomExcerptCleanupWorker {
    pub(crate) fn start(cleanup: Arc<dyn RoomExcerptCleanupUseCases>) -> Self {
        Self::start_every(cleanup, INTERVAL)
    }

    fn start_every(cleanup: Arc<dyn RoomExcerptCleanupUseCases>, period: Duration) -> Self {
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
                code = "room_excerpt.cleanup_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "分享的一段对话的定时清理异常结束"
            );
        }
    }
}

async fn run_worker(
    cleanup: Arc<dyn RoomExcerptCleanupUseCases>,
    period: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticks = interval(period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => match cleanup.prune().await {
                Ok(0) => {}
                Ok(deleted) => tracing::info!(deleted, "清理了一轮分享的一段对话"),
                Err(failure) => tracing::warn!(
                    operation = failure.operation(),
                    kind = ?failure.kind(),
                    "这次没清理成分享的一段对话，明天再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("分享的一段对话的定时清理已停止");
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
        ports::PortFuture,
        room_excerpts::{ExcerptResult, RoomExcerptCleanupUseCases},
    };
    use tokio::sync::Notify;

    use super::RoomExcerptCleanupWorker;

    #[derive(Default)]
    struct RecordingCleanup {
        calls: AtomicUsize,
        called: Notify,
    }

    impl RoomExcerptCleanupUseCases for RecordingCleanup {
        fn prune(&self) -> PortFuture<'_, ExcerptResult<u64>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(2)
            })
        }
    }

    #[tokio::test]
    async fn 启动就清一次_之后一轮一轮地清_关掉以后不再跑() {
        let cleanup = Arc::new(RecordingCleanup::default());
        let called = cleanup.called.notified();
        let worker = RoomExcerptCleanupWorker::start_every(cleanup.clone(), Duration::from_mins(1));
        tokio::time::timeout(Duration::from_secs(1), called)
            .await
            .expect("启动就清一次");
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), 1);
        worker.shutdown().await;

        let cleanup = Arc::new(RecordingCleanup::default());
        let worker =
            RoomExcerptCleanupWorker::start_every(cleanup.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
        worker.shutdown().await;
        let calls_after_shutdown = cleanup.calls.load(Ordering::SeqCst);
        assert!(calls_after_shutdown >= 2, "{calls_after_shutdown}");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
