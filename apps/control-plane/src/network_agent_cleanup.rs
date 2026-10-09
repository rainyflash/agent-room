//! 与 HTTP 服务共同存活、共同关闭的网络 Agent 定时清理：每分钟一轮，互不重叠；
//! 过了房间保留期的消息副本启动时删一次，之后每小时一次。只在总开关打开时启动。

use std::{sync::Arc, time::Duration};

use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

use crate::network_gateway::{NetworkAgentCleanupOutcome, NetworkGateway};

const INTERVAL: Duration = Duration::from_mins(1);
const PRUNE_INTERVAL: Duration = Duration::from_hours(1);

pub(crate) struct NetworkAgentCleanupWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl NetworkAgentCleanupWorker {
    pub(crate) fn start(gateway: Arc<NetworkGateway>) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(gateway, stop_requested));
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
                code = "network_agent.cleanup_worker_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "网络 Agent 定时清理异常结束"
            );
        }
    }
}

async fn run_worker(gateway: Arc<NetworkGateway>, mut stop_requested: oneshot::Receiver<()>) {
    let mut prune = interval(PRUNE_INTERVAL);
    prune.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = prune.tick() => prune_expired(&gateway).await,
            () = tokio::time::sleep(INTERVAL) => match gateway.clean_up().await {
                Ok(outcome) if outcome == NetworkAgentCleanupOutcome::default() => {}
                Ok(outcome) => tracing::info!(
                    disabled = outcome.disabled,
                    left = outcome.left,
                    abandoned = outcome.abandoned,
                    retrying = outcome.retrying,
                    keys_deleted = outcome.keys_deleted,
                    closed = outcome.closed,
                    "网络 Agent 清理了一轮"
                ),
                Err(failure) => tracing::warn!(
                    kind = ?failure.kind(),
                    "网络 Agent 这一轮清理没做完，下一轮再来"
                ),
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("网络 Agent 定时清理已停止");
}

async fn prune_expired(gateway: &NetworkGateway) {
    match gateway.prune_expired_messages().await {
        Ok(0) => {}
        Ok(pruned) => tracing::info!(pruned, "删掉了过了房间保留期的网络 Agent 消息副本"),
        Err(error) => tracing::warn!(
            kind = ?error.kind(),
            "这次没删成过了房间保留期的网络 Agent 消息副本，一小时后再来"
        ),
    }
}
