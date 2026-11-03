//! 发现页的“最近有人说话”（`specs/public-rooms/design.md`）：每 5 分钟以应用服务读一次每间公开房间
//! 最新一条消息事件的时间。与 HTTP 服务共同存活、共同关闭，启动时读一次。

use std::{sync::Arc, time::Duration};

use agent_room_application::public_room_activity::PublicRoomActivityUseCases;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

const INTERVAL: Duration = Duration::from_mins(5);

pub(crate) struct PublicRoomActivityWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl PublicRoomActivityWorker {
    pub(crate) fn start(activity: Arc<dyn PublicRoomActivityUseCases>) -> Self {
        Self::start_every(activity, INTERVAL)
    }

    fn start_every(activity: Arc<dyn PublicRoomActivityUseCases>, period: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(activity, period, stop_requested));
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
                code = "public_room.activity_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "读公开房间最近说话时间的定时任务异常结束"
            );
        }
    }
}

async fn run_worker(
    activity: Arc<dyn PublicRoomActivityUseCases>,
    period: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticks = interval(period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => match activity.refresh().await {
                Ok(round) if round.unreadable == 0 => tracing::debug!(
                    checked = round.checked,
                    recorded = round.recorded,
                    "读了一轮公开房间最近说话的时间"
                ),
                // 单间读不了多半是房间刚关、正在归档，下一轮就不在名单里了。
                Ok(round) => tracing::info!(
                    checked = round.checked,
                    recorded = round.recorded,
                    unreadable = round.unreadable,
                    "有几间公开房间这一轮没读到最近说话的时间"
                ),
                Err(error) => tracing::warn!(
                    operation = error.operation(),
                    kind = ?error.kind(),
                    "这一轮没读成公开房间最近说话的时间，5 分钟后再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("读公开房间最近说话时间的定时任务已停止");
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
        public_room_activity::{PublicRoomActivityRound, PublicRoomActivityUseCases},
    };
    use tokio::sync::Notify;

    use super::PublicRoomActivityWorker;

    #[derive(Default)]
    struct RecordingActivity {
        calls: AtomicUsize,
        called: Notify,
    }

    impl PublicRoomActivityUseCases for RecordingActivity {
        fn refresh(&self) -> PortFuture<'_, Result<PublicRoomActivityRound, RepositoryError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(PublicRoomActivityRound {
                    checked: 2,
                    recorded: 1,
                    unreadable: 1,
                })
            })
        }
    }

    #[tokio::test]
    async fn 启动就读一次_之后一轮一轮地读_关掉以后不再跑() {
        let activity = Arc::new(RecordingActivity::default());
        let called = activity.called.notified();
        let worker =
            PublicRoomActivityWorker::start_every(activity.clone(), Duration::from_mins(1));
        tokio::time::timeout(Duration::from_secs(1), called)
            .await
            .expect("启动就读一次");
        assert_eq!(activity.calls.load(Ordering::SeqCst), 1);
        worker.shutdown().await;

        let activity = Arc::new(RecordingActivity::default());
        let worker =
            PublicRoomActivityWorker::start_every(activity.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
        worker.shutdown().await;
        let calls_after_shutdown = activity.calls.load(Ordering::SeqCst);
        assert!(calls_after_shutdown >= 2, "{calls_after_shutdown}");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(activity.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
