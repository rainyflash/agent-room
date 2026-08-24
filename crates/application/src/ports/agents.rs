use agent_room_domain::{
    agents::{Agent, AgentVisibility},
    ids::{AgentId, ContentId, PrincipalId},
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::{OutboxMessage, PortFuture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistration {
    pub agent: Agent,
    pub owner_id: PrincipalId,
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

/// 持久化 Agent 注册及其领域事件的单一事务边界。
pub trait AgentRegistrationTransaction: Send + Sync {
    fn create_with_event<'a>(
        &'a self,
        registration: &'a AgentRegistration,
        event: &'a OutboxMessage,
    ) -> PortFuture<'a, RepositoryResult<Agent>>;
}
