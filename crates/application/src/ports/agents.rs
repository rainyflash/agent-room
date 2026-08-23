use agent_room_domain::{DomainResult, agents::Agent, ids::AgentId};

use super::PortFuture;

pub trait AgentRepository: Send + Sync {
    fn find(&self, id: AgentId) -> PortFuture<'_, DomainResult<Option<Agent>>>;

    fn save<'a>(&'a self, agent: &'a Agent) -> PortFuture<'a, DomainResult<()>>;
}
