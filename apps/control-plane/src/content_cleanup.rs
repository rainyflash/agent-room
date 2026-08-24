use std::{error::Error, fmt, sync::Arc, time::Duration};

use agent_room_application::content::ContentCleanupUseCases;
use tokio::{sync::oneshot, task::JoinHandle};

/// 与 HTTP 服务共同存活、共同关闭的内容回收执行器。
pub(crate) struct ContentCleanupWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ContentCleanupWorker {
    /// 启动单实例、非重叠执行的周期回收任务。
    ///
    /// # Errors
    ///
    /// 周期为零时拒绝启动，防止错误配置制造忙循环。
    pub(crate) fn start(
        cleanup: Arc<dyn ContentCleanupUseCases>,
        interval: Duration,
    ) -> Result<Self, ContentCleanupWorkerError> {
        if interval.is_zero() {
            return Err(ContentCleanupWorkerError::ZeroInterval);
        }
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(cleanup, interval, stop_requested));
        Ok(Self {
            stop: Some(stop),
            task,
        })
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Err(error) = self.task.await {
            tracing::error!(
                code = "content.cleanup_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "内容回收执行器异常结束"
            );
        }
    }
}

async fn run_worker(
    cleanup: Arc<dyn ContentCleanupUseCases>,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(interval) => {
                match cleanup.run_cleanup().await {
                    Ok(outcome) => {
                        tracing::info!(
                            examined = outcome.examined,
                            deleted = outcome.deleted,
                            failures = outcome.failures.len(),
                            "内容回收批次已完成"
                        );
                    }
                    Err(error) => {
                        tracing::error!(
                            code = "content.cleanup_batch_failed",
                            error = ?error,
                            "内容回收批次失败"
                        );
                    }
                }
            }
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("内容回收执行器已停止");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentCleanupWorkerError {
    ZeroInterval,
}

impl fmt::Display for ContentCleanupWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInterval => formatter.write_str("内容回收周期必须大于零"),
        }
    }
}

impl Error for ContentCleanupWorkerError {}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use agent_room_application::{
        content::{CleanupContentOutcome, CleanupContentResult, ContentCleanupUseCases},
        ports::PortFuture,
    };
    use tokio::sync::Notify;

    use super::ContentCleanupWorker;

    struct RecordingCleanup {
        calls: AtomicUsize,
        called: Notify,
    }

    impl RecordingCleanup {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                called: Notify::new(),
            }
        }
    }

    impl ContentCleanupUseCases for RecordingCleanup {
        fn run_cleanup(&self) -> PortFuture<'_, CleanupContentResult<CleanupContentOutcome>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                Ok(CleanupContentOutcome {
                    examined: 1,
                    deleted: 1,
                    failures: Vec::new(),
                })
            })
        }
    }

    #[test]
    fn 拒绝零周期避免忙循环() {
        let cleanup = Arc::new(RecordingCleanup::new());
        assert!(ContentCleanupWorker::start(cleanup, Duration::ZERO).is_err());
    }

    #[tokio::test]
    async fn 周期执行并在关闭后停止() {
        let cleanup = Arc::new(RecordingCleanup::new());
        let worker = ContentCleanupWorker::start(cleanup.clone(), Duration::from_millis(10))
            .expect("有效周期应启动执行器");

        tokio::time::timeout(Duration::from_secs(1), cleanup.called.notified())
            .await
            .expect("执行器应在周期内运行一次");
        worker.shutdown().await;
        let calls_after_shutdown = cleanup.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert_eq!(cleanup.calls.load(Ordering::SeqCst), calls_after_shutdown);
    }
}
