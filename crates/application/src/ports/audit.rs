use agent_room_domain::{DomainResult, ids::PrincipalId, time::UtcMillis};

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub actor_id: PrincipalId,
    pub action: String,
    pub occurred_at: UtcMillis,
    pub correlation_id: String,
}

pub trait AuditSink: Send + Sync {
    fn append<'a>(&'a self, record: &'a AuditRecord) -> PortFuture<'a, DomainResult<()>>;
}
