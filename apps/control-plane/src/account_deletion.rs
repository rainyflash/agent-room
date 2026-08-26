use std::{error::Error, fmt, sync::Arc, time::Duration};

use agent_room_application::account_lifecycle::{
    AccountDeletionWorker, AccountDeletionWorkerOutcome,
};
use tokio::{sync::oneshot, task::JoinHandle, time::MissedTickBehavior};

/// 与控制平面共同存活、共同关闭的账户删除 Saga 执行器。
pub(crate) struct AccountDeletionRuntime {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl AccountDeletionRuntime {
    /// 启动单实例、非重叠执行的账户删除循环。
    ///
    /// # Errors
    ///
    /// 周期为零时拒绝启动，避免错误配置制造忙循环。
    pub(crate) fn start(
        worker: Arc<AccountDeletionWorker>,
        interval: Duration,
    ) -> Result<Self, AccountDeletionRuntimeError> {
        if interval.is_zero() {
            return Err(AccountDeletionRuntimeError::ZeroInterval);
        }
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(worker, interval, stop_requested));
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
                code = "account.deletion_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "账户删除执行器异常结束"
            );
        }
    }
}

async fn run_worker(
    worker: Arc<AccountDeletionWorker>,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => match worker.run_once().await {
                Ok(AccountDeletionWorkerOutcome::Idle) => {}
                Ok(AccountDeletionWorkerOutcome::Completed(job_id)) => {
                    tracing::info!(job.id = %job_id, "账户删除工作流已完成");
                }
                Ok(AccountDeletionWorkerOutcome::Retrying(job_id)) => {
                    tracing::warn!(job.id = %job_id, "账户删除外部步骤将在退避后重试");
                }
                Err(error) => {
                    tracing::error!(
                        code = "account.deletion_batch_failed",
                        operation = error.operation(),
                        failure = ?error.kind(),
                        "账户删除批次失败"
                    );
                }
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("账户删除执行器已停止");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountDeletionRuntimeError {
    ZeroInterval,
}

impl fmt::Display for AccountDeletionRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInterval => formatter.write_str("账户删除执行周期必须大于零"),
        }
    }
}

impl Error for AccountDeletionRuntimeError {}
