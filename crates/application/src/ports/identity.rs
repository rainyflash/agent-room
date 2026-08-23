use agent_room_domain::{DomainResult, identity::Principal, ids::PrincipalId};

use super::PortFuture;

pub trait PrincipalRepository: Send + Sync {
    fn find(&self, id: PrincipalId) -> PortFuture<'_, DomainResult<Option<Principal>>>;

    fn save<'a>(&'a self, principal: &'a Principal) -> PortFuture<'a, DomainResult<()>>;
}
