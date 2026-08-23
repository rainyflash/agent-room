use agent_room_domain::{
    identity::Principal,
    ids::{ContentId, PrincipalId},
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalRegistration {
    pub principal: Principal,
    pub oidc_issuer: String,
    pub oidc_subject: String,
    pub matrix_user_id: String,
    pub display_name: String,
    pub avatar_content_id: Option<ContentId>,
    pub locale: String,
    pub registered_at: UtcMillis,
}

pub trait PrincipalRepository: Send + Sync {
    fn find(&self, id: PrincipalId) -> PortFuture<'_, RepositoryResult<Option<Principal>>>;

    fn create<'a>(
        &'a self,
        registration: &'a PrincipalRegistration,
    ) -> PortFuture<'a, RepositoryResult<Principal>>;

    fn save<'a>(&'a self, principal: &'a Principal) -> PortFuture<'a, RepositoryResult<Principal>>;
}
