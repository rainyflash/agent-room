use agent_room_domain::{DomainResult, ids::PrincipalId};

use super::PortFuture;

pub trait NotificationSink: Send + Sync {
    fn notify<'a>(
        &'a self,
        principal_id: PrincipalId,
        title: &'a str,
        body: &'a str,
    ) -> PortFuture<'a, DomainResult<()>>;
}
