use agent_room_domain::{
    DomainResult,
    ids::{AgentInstanceId, RoomInstanceId},
};

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixEvent {
    pub event_type: String,
    pub transaction_id: String,
    pub payload: Vec<u8>,
}

pub trait MatrixGateway: Send + Sync {
    fn send<'a>(
        &'a self,
        room_id: RoomInstanceId,
        actor_id: AgentInstanceId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, DomainResult<String>>;
}
