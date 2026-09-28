use std::sync::{Arc, Weak};

use agent_room_application::ports::MatrixRoomId;
use agent_room_bridge_core::status::{
    AgentStatusIntent, AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState,
    StatusPublicationOutcome, StatusPublicationResult, WAIT_IDLE_TIMEOUT,
};
use agent_room_domain::time::UtcMillis;
use tokio::{sync::Mutex, time::Instant};
use uuid::Uuid;

pub(crate) struct AgentStatusPublicationHandle {
    target: AgentStatusRoomTarget,
    state: Mutex<AgentStatusPublicationState>,
}

struct AgentStatusPublicationState {
    service: AgentStatusPublicationService,
    intent: AgentStatusIntent,
    /// 最近一次空手而归的 `WaitInbox`。等待的进程被杀或取消后就不会再来，看门狗据此清除等待。
    last_wait_at: Option<Instant>,
    watchdog_running: bool,
}

impl AgentStatusPublicationHandle {
    pub(crate) fn new(
        service: AgentStatusPublicationService,
        target: AgentStatusRoomTarget,
        initial_state: HostAgentState,
    ) -> Self {
        Self {
            target,
            state: Mutex::new(AgentStatusPublicationState {
                service,
                intent: AgentStatusIntent::new(initial_state, None),
                last_wait_at: None,
                watchdog_running: false,
            }),
        }
    }

    pub(crate) async fn publish(
        &self,
        host_state: HostAgentState,
    ) -> StatusPublicationResult<StatusPublicationOutcome> {
        let mut state = self.state.lock().await;
        let connected = host_state != HostAgentState::Disconnected;
        let intent = AgentStatusIntent::new(host_state, None)
            .with_last_polled_at(state.intent.last_polled_at().filter(|_| connected))
            .with_waiting(connected && state.intent.waiting());
        let outcome = state
            .service
            .publish_if_due(&self.target, &intent, status_entropy())
            .await?;
        state.intent = intent;
        Ok(outcome)
    }

    pub(crate) async fn renew(&self) -> StatusPublicationResult<StatusPublicationOutcome> {
        let mut state = self.state.lock().await;
        let intent = state.intent.clone();
        state
            .service
            .publish_if_due(&self.target, &intent, status_entropy())
            .await
    }

    /// 宿主收件后记下读取时间与是否仍在等待。开始、结束等待各发一次状态；等待期间不按轮询重发，
    /// 由看门狗在 [`WAIT_IDLE_TIMEOUT`] 内没再等时清除。
    pub(crate) async fn note_inbox_wait(
        self: &Arc<Self>,
        room_id: &MatrixRoomId,
        at: UtcMillis,
        waiting: bool,
    ) -> StatusPublicationResult<()> {
        // A private-room fetch cannot advertise reception in the public lobby.
        if room_id != self.target.room_id() {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        state.intent = state
            .intent
            .clone()
            .with_last_polled_at(Some(at))
            .with_waiting(waiting);
        state.last_wait_at = waiting.then(Instant::now);
        if waiting && !state.watchdog_running {
            state.watchdog_running = true;
            tokio::spawn(watch_abandoned_wait(Arc::downgrade(self)));
        }
        let intent = state.intent.clone();
        state
            .service
            .publish_if_due(&self.target, &intent, status_entropy())
            .await?;
        Ok(())
    }
}

/// 等待的进程被杀、被取消时不会告诉 Bridge，只是不再来 `WaitInbox`。隔了 [`WAIT_IDLE_TIMEOUT`]
/// 还没来就清除等待，不让“持续等待消息”一直挂到 `waitingUntil`。
async fn watch_abandoned_wait(handle: Weak<AgentStatusPublicationHandle>) {
    loop {
        let Some(handle) = handle.upgrade() else {
            return;
        };
        let mut state = handle.state.lock().await;
        let Some(last_wait_at) = state.last_wait_at.filter(|_| state.intent.waiting()) else {
            state.watchdog_running = false;
            return;
        };
        let deadline = last_wait_at + WAIT_IDLE_TIMEOUT;
        if Instant::now() < deadline {
            drop(state);
            drop(handle);
            tokio::time::sleep_until(deadline).await;
            continue;
        }
        state.intent = state.intent.clone().with_waiting(false);
        state.last_wait_at = None;
        state.watchdog_running = false;
        let intent = state.intent.clone();
        // 没发出去也不要紧：下一次续租按清除后的意图重发。
        if let Err(failure) = state
            .service
            .publish_if_due(&handle.target, &intent, status_entropy())
            .await
        {
            tracing::warn!(kind = ?failure.kind(), "could not clear an abandoned inbox wait");
        }
        return;
    }
}

fn status_entropy() -> u64 {
    let bytes = Uuid::now_v7().into_bytes();
    u64::from_le_bytes(bytes[8..].try_into().expect("UUID 后八字节长度固定"))
}
