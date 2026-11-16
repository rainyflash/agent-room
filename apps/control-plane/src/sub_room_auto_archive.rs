//! 子房间 30 天没人说话自动归档（`specs/redesign/sub-rooms.md` 第 2j-2 步）：每小时一轮。与 HTTP 服务
//! 共同存活、共同关闭，启动时做一轮。多个控制面同时跑时靠租期分开，同一间不做两遍。

use std::{sync::Arc, time::Duration};

use agent_room_application::sub_room_auto_archive::SubRoomAutoArchiveUseCases;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

const INTERVAL: Duration = Duration::from_hours(1);

pub(crate) struct SubRoomAutoArchiveWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl SubRoomAutoArchiveWorker {
    pub(crate) fn start(auto_archive: Arc<dyn SubRoomAutoArchiveUseCases>) -> Self {
        Self::start_every(auto_archive, INTERVAL)
    }

    fn start_every(auto_archive: Arc<dyn SubRoomAutoArchiveUseCases>, period: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(auto_archive, period, stop_requested));
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
                code = "sub_room.auto_archive_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "子房间自动归档的定时任务异常结束"
            );
        }
    }
}

async fn run_worker(
    auto_archive: Arc<dyn SubRoomAutoArchiveUseCases>,
    period: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticks = interval(period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => match auto_archive.archive_quiet().await {
                Ok(round) if round.unreadable == 0 && round.failed == 0 => tracing::debug!(
                    claimed = round.claimed,
                    archived = round.archived,
                    still_talking = round.still_talking,
                    "做了一轮子房间自动归档"
                ),
                // 单间读不了、存不进多半是聊天服务器一时不通或者正好有人在改它，租期到了下一轮再来。
                Ok(round) => tracing::info!(
                    claimed = round.claimed,
                    archived = round.archived,
                    still_talking = round.still_talking,
                    unreadable = round.unreadable,
                    failed = round.failed,
                    "有几间子房间这一轮没做成自动归档，下一轮再来"
                ),
                Err(error) => tracing::warn!(
                    operation = error.operation(),
                    kind = ?error.kind(),
                    "这一轮没领到要自动归档的子房间，一小时后再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("子房间自动归档的定时任务已停止");
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
        sub_room_auto_archive::{SubRoomAutoArchiveRound, SubRoomAutoArchiveUseCases},
    };
    use tokio::sync::Notify;

    use super::SubRoomAutoArchiveWorker;

    #[derive(Default)]
    struct RecordingAutoArchive {
        calls: AtomicUsize,
        called: Notify,
    }

    impl SubRoomAutoArchiveUseCases for RecordingAutoArchive {
        fn archive_quiet(
            &self,
        ) -> PortFuture<'_, Result<SubRoomAutoArchiveRound, RepositoryError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(SubRoomAutoArchiveRound {
                    claimed: 2,
                    archived: 1,
                    unreadable: 1,
                    ..SubRoomAutoArchiveRound::default()
                })
            })
        }
    }

    #[tokio::test]
    async fn 启动就做一轮_之后一轮一轮地做_关掉以后不再跑() {
        let auto_archive = Arc::new(RecordingAutoArchive::default());
        let called = auto_archive.called.notified();
        let worker =
            SubRoomAutoArchiveWorker::start_every(auto_archive.clone(), Duration::from_mins(1));
        tokio::time::timeout(Duration::from_secs(1), called)
            .await
            .expect("启动就做一轮");
        assert_eq!(auto_archive.calls.load(Ordering::SeqCst), 1);
        worker.shutdown().await;

        let auto_archive = Arc::new(RecordingAutoArchive::default());
        let worker =
            SubRoomAutoArchiveWorker::start_every(auto_archive.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
        worker.shutdown().await;
        let calls_after_shutdown = auto_archive.calls.load(Ordering::SeqCst);
        assert!(calls_after_shutdown >= 2, "{calls_after_shutdown}");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            auto_archive.calls.load(Ordering::SeqCst),
            calls_after_shutdown
        );
    }
}
