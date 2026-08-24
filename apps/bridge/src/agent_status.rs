use agent_room_bridge_core::status::{
    AgentStatusIntent, AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState,
    StatusPublicationOutcome, StatusPublicationResult,
};
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
        let intent = AgentStatusIntent::new(host_state, None);
        let mut state = self.state.lock().await;
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
}

fn status_entropy() -> u64 {
    let bytes = Uuid::now_v7().into_bytes();
    u64::from_le_bytes(bytes[8..].try_into().expect("UUID 后八字节长度固定"))
}
