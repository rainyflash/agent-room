use agent_room_domain::{
    agents::{Agent, AgentVisibility},
    ids::{AgentId, ContentId},
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistration {
    pub agent: Agent,
    pub matrix_user_id: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub avatar_content_id: Option<ContentId>,
    pub visibility: AgentVisibility,
    pub registered_at: UtcMillis,
}

pub trait AgentRepository: Send + Sync {
    fn find(&self, id: AgentId) -> PortFuture<'_, RepositoryResult<Option<Agent>>>;

    fn create<'a>(
        &'a self,
        registration: &'a AgentRegistration,
    ) -> PortFuture<'a, RepositoryResult<Agent>>;

    fn save<'a>(&'a self, agent: &'a Agent) -> PortFuture<'a, RepositoryResult<Agent>>;
}
