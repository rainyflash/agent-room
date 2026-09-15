use agent_room_application::ports::MatrixRoomId;
use agent_room_bridge_core::status::{
    AgentStatusIntent, AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState,
    StatusPublicationOutcome, StatusPublicationResult,
};
use agent_room_domain::time::UtcMillis;
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) struct AgentStatusPublicationHandle {
    target: AgentStatusRoomTarget,
    state: Mutex<AgentStatusPublicationState>,
}

struct AgentStatusPublicationState {
    service: AgentStatusPublicationService,
    intent: AgentStatusIntent,
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
            }),
        }
    }

    pub(crate) async fn publish(
        &self,
        host_state: HostAgentState,
    ) -> StatusPublicationResult<StatusPublicationOutcome> {
        let mut state = self.state.lock().await;
        let last_polled_at = if host_state == HostAgentState::Disconnected {
            None
        } else {
            state.intent.last_polled_at()
        };
        let intent = AgentStatusIntent::new(host_state, None)
            .with_last_polled_at(last_polled_at)
            .with_listening_until(if host_state == HostAgentState::Disconnected {
                None
            } else {
                state.intent.listening_until()
            });
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

    pub(crate) async fn note_inbox_wait(
        &self,
        room_id: &MatrixRoomId,
        at: UtcMillis,
        waiting: bool,
    ) -> StatusPublicationResult<()> {
        // A private-room fetch cannot advertise reception in the public lobby.
        if room_id != self.target.room_id() {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        let until = waiting.then(|| {
            UtcMillis::new(
                at.value()
                    .saturating_add(agent_room_domain::agent_lifecycle::RECEPTION_FRESHNESS_MS),
            )
            .expect("非负时间加饱和期限仍为非负时间")
        });
        state.intent = state
            .intent
            .clone()
            .with_last_polled_at(Some(at))
            .with_listening_until(until);
        let intent = state.intent.clone();
        state
            .service
            .publish_if_due(&self.target, &intent, status_entropy())
            .await?;
        Ok(())
    }
}

fn status_entropy() -> u64 {
    let bytes = Uuid::now_v7().into_bytes();
    u64::from_le_bytes(bytes[8..].try_into().expect("UUID 后八字节长度固定"))
}
