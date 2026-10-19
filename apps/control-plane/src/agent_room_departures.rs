//! 撤销电脑以后它的 Agent 离开房间（`specs/computer-revocation/design.md`）：撤销的那次请求里先做一遍，
//! 没做成的由这里接着做。与 HTTP 服务共同存活、共同关闭，每 30 秒一轮，互不重叠。一条一条地领，多个
//! 副本不会同时做同一条；没做成的往后退避再试。

use std::{sync::Arc, time::Duration};

use agent_room_application::agent_room_departures::{AgentRoomDepartureUseCases, DepartureRound};
use agent_room_domain::ids::DeviceId;
use tokio::{sync::oneshot, task::JoinHandle};

const INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct AgentRoomDepartureWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl AgentRoomDepartureWorker {
    pub(crate) fn start(departures: Arc<dyn AgentRoomDepartureUseCases>) -> Self {
        Self::start_every(departures, INTERVAL)
    }

    fn start_every(departures: Arc<dyn AgentRoomDepartureUseCases>, interval: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(departures, interval, stop_requested));
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
                code = "agent_room.departure_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "撤销电脑以后离开房间的定时任务异常结束"
            );
        }
    }
}

/// 撤销的那次请求里接着做这台电脑记下的。没做成的不挡撤销的回答，留给定时任务。
pub(crate) async fn depart_after_revocation(
    departures: &dyn AgentRoomDepartureUseCases,
    device_id: DeviceId,
) {
    match departures.depart_for_device(device_id).await {
        Ok(round) => report(&round),
        Err(failure) => tracing::warn!(
            operation = failure.operation(),
            kind = ?failure.kind(),
            device.id = %device_id,
            "撤销电脑以后没能马上让它的 Agent 离开房间，交给定时任务"
        ),
    }
}

async fn run_worker(
    departures: Arc<dyn AgentRoomDepartureUseCases>,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(interval) => match departures.depart_due().await {
                Ok(round) => report(&round),
                Err(failure) => tracing::warn!(
                    operation = failure.operation(),
                    kind = ?failure.kind(),
                    "撤销电脑以后离开房间这一轮没做完，下一轮再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("撤销电脑以后离开房间的定时任务已停止");
}

fn report(round: &DepartureRound) {
    if round.left > 0 || round.released_only > 0 {
        tracing::info!(
            left = round.left,
            released_only = round.released_only,
            "撤销电脑以后，它的 Agent 离开了一批房间"
        );
    }
    for retry in &round.retrying {
        tracing::warn!(
            agent.id = %retry.agent_id,
            room_instance.id = %retry.room_instance_id,
            code = retry.failure_code,
            "撤销电脑以后 Agent 这一轮没离开房间，往后退避再试"
        );
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
        agent_room_departures::{AgentRoomDepartureUseCases, DepartureRound},
        persistence::RepositoryError,
        ports::PortFuture,
    };
    use agent_room_domain::ids::DeviceId;
    use tokio::sync::Notify;

    use super::AgentRoomDepartureWorker;

    #[derive(Default)]
    struct RecordingDepartures {
        calls: AtomicUsize,
        called: Notify,
    }

    impl AgentRoomDepartureUseCases for RecordingDepartures {
        fn depart_for_device(
            &self,
            _device_id: DeviceId,
        ) -> PortFuture<'_, Result<DepartureRound, RepositoryError>> {
            Box::pin(async { unreachable!("定时任务只做到期的") })
        }

        fn depart_due(&self) -> PortFuture<'_, Result<DepartureRound, RepositoryError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(DepartureRound {
                    left: 1,
                    ..DepartureRound::default()
                })
            })
        }
    }

    #[tokio::test]
    async fn 一轮一轮地做_关掉以后不再跑() {
        let departures = Arc::new(RecordingDepartures::default());
        let worker =
            AgentRoomDepartureWorker::start_every(departures.clone(), Duration::from_millis(10));

        tokio::time::timeout(Duration::from_secs(1), departures.called.notified())
            .await
            .expect("一个周期内应跑一轮");
        worker.shutdown().await;
        let calls_after_shutdown = departures.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert_eq!(
            departures.calls.load(Ordering::SeqCst),
            calls_after_shutdown
        );
    }
}
