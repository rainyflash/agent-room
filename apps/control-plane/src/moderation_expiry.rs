//! 治理动作到期自动解除：与 HTTP 服务共同存活、共同关闭，每 30 秒一轮，互不重叠。没撤成的动作保持
//! 已生效，下一轮再试。

use std::{sync::Arc, time::Duration};

use agent_room_application::moderation::{ModerationExpiryOutcome, ModerationExpiryUseCases};
use tokio::{sync::oneshot, task::JoinHandle};

const INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct ModerationExpiryWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ModerationExpiryWorker {
    pub(crate) fn start(expiry: Arc<dyn ModerationExpiryUseCases>) -> Self {
        Self::start_every(expiry, INTERVAL)
    }

    fn start_every(expiry: Arc<dyn ModerationExpiryUseCases>, interval: Duration) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(expiry, interval, stop_requested));
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
                code = "moderation.expiry_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "治理动作到期解除异常结束"
            );
        }
    }
}

async fn run_worker(
    expiry: Arc<dyn ModerationExpiryUseCases>,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(interval) => match expiry.expire_due_actions().await {
                Ok(outcome) => report(&outcome),
                Err(failure) => tracing::warn!(
                    operation = failure.operation(),
                    kind = ?failure.kind(),
                    "治理动作到期解除这一轮没做完，下一轮再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("治理动作到期解除已停止");
}

fn report(outcome: &ModerationExpiryOutcome) {
    if outcome.expired > 0 {
        tracing::info!(expired = outcome.expired, "到期的治理动作解除了一批");
    }
    for retry in &outcome.retrying {
        tracing::warn!(
            moderation_action.id = %retry.action_id,
            code = retry.failure_code,
            "到期的治理动作这一轮没解除成，下一轮再试"
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
        moderation::{ModerationExpiryOutcome, ModerationExpiryUseCases, ModerationResult},
        ports::PortFuture,
    };
    use tokio::sync::Notify;

    use super::ModerationExpiryWorker;

    #[derive(Default)]
    struct RecordingExpiry {
        calls: AtomicUsize,
        called: Notify,
    }

    impl ModerationExpiryUseCases for RecordingExpiry {
        fn expire_due_actions(&self) -> PortFuture<'_, ModerationResult<ModerationExpiryOutcome>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(ModerationExpiryOutcome {
                    expired: 1,
                    retrying: Vec::new(),
                })
            })
        }
    }

    #[tokio::test]
    async fn 一轮一轮地解除_关掉以后不再跑() {
        let expiry = Arc::new(RecordingExpiry::default());
        let worker = ModerationExpiryWorker::start_every(expiry.clone(), Duration::from_millis(10));

        tokio::time::timeout(Duration::from_secs(1), expiry.called.notified())
            .await
            .expect("一个周期内应跑一轮");
        worker.shutdown().await;
        let calls_after_shutdown = expiry.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert_eq!(expiry.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
