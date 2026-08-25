use agent_room_domain::ids::{AgentId, RoomCatalogId};

use crate::{
    authentication::AuthenticatedPrincipal,
    ports::{DirectAgentProfile, DirectSessionRecord},
};
use agent_room_domain::direct_sessions::DirectContactPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDirectSession {
    pub actor: AuthenticatedPrincipal,
    pub target_agent_id: AgentId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectDirectSession {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListDirectSessions {
    pub actor: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetDirectAgentBlock {
    pub actor: AuthenticatedPrincipal,
    pub target_agent_id: AgentId,
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectSessionView {
    pub record: DirectSessionRecord,
    pub target: DirectAgentProfile,
    pub contact_policy: DirectContactPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectContactView {
    pub target: DirectAgentProfile,
    pub contact_policy: DirectContactPolicy,
}
