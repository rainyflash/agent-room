//! 子房间在聊天服务器上跟着父房间变（`specs/redesign/sub-rooms.md` 的“对账”）：与 HTTP 服务共同存活、
//! 共同关闭，每 30 秒一轮，互不重叠。改父房间的请求结束前已经做过一遍，这里补上没做成的、请求里
//! 来不及做的，和回滚到旧版那段时间留下的。一件事一件事地领，多个副本不会同时做同一件。

use std::{sync::Arc, time::Duration};

use agent_room_application::private_rooms::{SubRoomSyncOutcome, SubRoomSyncUseCases};
use tokio::{sync::oneshot, task::JoinHandle};

const INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct SubRoomSyncWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl SubRoomSyncWorker {
    pub(crate) fn start(sync: Arc<dyn SubRoomSyncUseCases>) -> Self {
        Self::start_every(sync, INTERVAL)
    }

    fn start_every(sync: Arc<dyn SubRoomSyncUseCases>, interval: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(sync, interval, stop_requested));
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
                code = "sub_room.sync_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "子房间跟着父房间变的后台异常结束"
            );
        }
    }
}

async fn run_worker(
    sync: Arc<dyn SubRoomSyncUseCases>,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(interval) => match sync.sync_sub_rooms().await {
                Ok(outcome) => report(&outcome),
                Err(failure) => tracing::warn!(
                    operation = failure.operation(),
                    stage = ?failure.stage(),
                    kind = ?failure.kind(),
                    "子房间跟着父房间变这一轮没做完，下一轮再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("子房间跟着父房间变的后台已停止");
}

fn report(outcome: &SubRoomSyncOutcome) {
    if outcome.followed > 0 {
        tracing::info!(
            followed = outcome.followed,
            "照到的父房间版本落后的子房间重算了一批"
        );
    }
    if outcome.done > 0 {
        tracing::info!(
            done = outcome.done,
            "子房间在聊天服务器上跟着父房间做了一批"
        );
    }
    for retry in &outcome.retrying {
        // 头一次没做成才记告警：之后每次退避再试都记的话，一件卡住的事会一直刷。
        if retry.attempt == 1 {
            tracing::warn!(
                sub_room_sync.id = retry.task_id,
                code = retry.failure_code,
                "子房间在聊天服务器上跟着父房间做的一件事没做成，往后退避再试"
            );
        } else {
            tracing::debug!(
                sub_room_sync.id = retry.task_id,
                code = retry.failure_code,
                attempt = retry.attempt,
                "子房间在聊天服务器上跟着父房间做的一件事还是没做成"
            );
        }
    }
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
        private_rooms::{
            PrivateRoomResult, SubRoomSyncOutcome, SubRoomSyncRetry, SubRoomSyncUseCases,
        },
    };
    use tokio::sync::Notify;

    use super::SubRoomSyncWorker;

    #[derive(Default)]
    struct RecordingSync {
        calls: AtomicUsize,
        called: Notify,
    }

    impl SubRoomSyncUseCases for RecordingSync {
        fn sync_sub_rooms(&self) -> PortFuture<'_, PrivateRoomResult<SubRoomSyncOutcome>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(SubRoomSyncOutcome {
                    followed: 1,
                    done: 1,
                    retrying: vec![
                        SubRoomSyncRetry {
                            task_id: 1,
                            failure_code: "matrix.unavailable",
                            attempt: 1,
                        },
                        SubRoomSyncRetry {
                            task_id: 2,
                            failure_code: "matrix.forbidden",
                            attempt: 4,
                        },
                    ],
                })
            })
        }
    }

    #[tokio::test]
    async fn 一轮一轮地做_关掉以后不再跑() {
        let sync = Arc::new(RecordingSync::default());
        let worker = SubRoomSyncWorker::start_every(sync.clone(), Duration::from_millis(10));

        tokio::time::timeout(Duration::from_secs(1), sync.called.notified())
            .await
            .expect("一个周期内应跑一轮");
        worker.shutdown().await;
        let calls_after_shutdown = sync.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert_eq!(sync.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
