//! 补问写名片的 Agent 的 Matrix 在线状态（`specs/agent-liveness/design.md`）。
//!
//! 全量同步只带不离线的人；写名片、却没在同步里出现的 Agent，各问一次
//! `GET /presence/{userId}/status`。放在后台问，不挡同步：大厅里离线的 Agent 可能有几百个。

use std::{sync::Arc, time::Duration};

use agent_room_application::ports::MatrixGateway;
use agent_room_bridge_core::presence::PresenceSyncService;
use tokio::{sync::Notify, task::JoinHandle, time::sleep};

/// 问失败的过一分钟再问，所以没人叫醒也一分钟看一次。
const IDLE_CHECK: Duration = Duration::from_mins(1);

pub(super) struct PresenceFetchWorker {
    task: JoinHandle<()>,
    wake: Arc<Notify>,
}

impl PresenceFetchWorker {
    pub(super) fn spawn(
        presence: Arc<PresenceSyncService>,
        matrix: Arc<dyn MatrixGateway>,
    ) -> Self {
        let wake = Arc::new(Notify::new());
        let notified = wake.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = notified.notified() => {}
                    () = sleep(IDLE_CHECK) => {}
                }
                match presence.fetch_missing_presence(matrix.as_ref()).await {
                    Ok(0) => {}
                    Ok(asked) => {
                        tracing::debug!(asked, "问了同步里没带的 Agent 的 Matrix 在线状态");
                    }
                    Err(failure) => {
                        tracing::warn!(
                            failure_kind = ?failure.kind(),
                            "没能记下 Agent 的 Matrix 在线状态"
                        );
                    }
                }
            }
        });
        Self { task, wake }
    }

    /// 同步完叫醒一次：新来的名片要去问。正在问的话，问完马上再看一遍。
    pub(super) fn wake(&self) {
        self.wake.notify_one();
    }

    pub(super) fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for PresenceFetchWorker {
    fn drop(&mut self) {
        self.task.abort();
    }
}
