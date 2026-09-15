use super::PortFuture;
use crate::persistence::RepositoryResult;
use agent_room_domain::ids::PrincipalId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRoom {
    pub catalog_id: String,
    pub room_id: String,
    pub name: String,
    pub direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxHandoff {
    pub handoff_id: String,
    pub room_id: String,
    pub message_id: String,
    pub agent_name: String,
    pub status: String,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalInboxIndex {
    pub rooms: Vec<InboxRoom>,
    pub handoffs: Vec<InboxHandoff>,
    pub limited: bool,
}

/// Account-scoped routing and delivery metadata. Message bodies stay in Matrix.
pub trait PersonalInboxRepository: Send + Sync {
    fn index(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<PersonalInboxIndex>>;
}
