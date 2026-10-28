//! 撤销以后 Matrix 设备没注销成的，后台再试（`specs/outbox/design.md` 第 4 步）。与 HTTP 服务共同
//! 存活、共同关闭，每 30 秒一轮，互不重叠。

use std::{sync::Arc, time::Duration};

use agent_room_application::agent_matrix_device_retry::{
    AgentMatrixDeviceRetryUseCases, MatrixDeviceRetryRound,
};
use tokio::{sync::oneshot, task::JoinHandle};

const INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct AgentMatrixDeviceRetryWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl AgentMatrixDeviceRetryWorker {
    pub(crate) fn start(retries: Arc<dyn AgentMatrixDeviceRetryUseCases>) -> Self {
        Self::start_every(retries, INTERVAL)
    }

    fn start_every(retries: Arc<dyn AgentMatrixDeviceRetryUseCases>, interval: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(retries, interval, stop_requested));
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
                code = "agent_instance.matrix_retry_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "撤销以后注销 Matrix 设备的定时任务异常结束"
            );
        }
    }
}

async fn run_worker(
    retries: Arc<dyn AgentMatrixDeviceRetryUseCases>,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(interval) => match retries.retry_due().await {
                Ok(round) => report(&round),
                Err(failure) => tracing::warn!(
                    operation = failure.operation(),
                    kind = ?failure.kind(),
                    "撤销以后注销 Matrix 设备这一轮没做完，下一轮再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("撤销以后注销 Matrix 设备的定时任务已停止");
}

fn report(round: &MatrixDeviceRetryRound) {
    if round.revoked > 0 {
        tracing::info!(
            revoked = round.revoked,
            "撤销以后没注销成的 Matrix 设备，这一轮注销了一批"
        );
    }
    for retry in &round.retrying {
        if retry.first_failure {
            tracing::warn!(
                agent_instance.id = %retry.instance_id,
                code = retry.failure_code,
                "撤销以后 Matrix 设备还没注销成，往后退避再试"
            );
        } else {
            tracing::debug!(
                agent_instance.id = %retry.instance_id,
                code = retry.failure_code,
                "撤销以后 Matrix 设备又没注销成，接着退避"
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
        agent_matrix_device_retry::{AgentMatrixDeviceRetryUseCases, MatrixDeviceRetryRound},
        persistence::RepositoryError,
        ports::PortFuture,
    };
    use tokio::sync::Notify;

    use super::AgentMatrixDeviceRetryWorker;

    #[derive(Default)]
    struct RecordingRetries {
        calls: AtomicUsize,
        called: Notify,
    }

    impl AgentMatrixDeviceRetryUseCases for RecordingRetries {
        fn retry_due(&self) -> PortFuture<'_, Result<MatrixDeviceRetryRound, RepositoryError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(MatrixDeviceRetryRound {
                    revoked: 1,
                    ..MatrixDeviceRetryRound::default()
                })
            })
        }
    }

    #[tokio::test]
    async fn 一轮一轮地做_关掉以后不再跑() {
        let retries = Arc::new(RecordingRetries::default());
        let worker =
            AgentMatrixDeviceRetryWorker::start_every(retries.clone(), Duration::from_millis(10));

        tokio::time::timeout(Duration::from_secs(1), retries.called.notified())
            .await
            .expect("一个周期内应跑一轮");
        worker.shutdown().await;
        let calls_after_shutdown = retries.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert_eq!(retries.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
