use agent_room_domain::{
    DomainResult,
    ids::{AgentInstanceId, RoomCatalogId},
    rooms::RoomAllocation,
};

use super::PortFuture;

pub trait RoomDirectory: Send + Sync {
    fn allocate(
        &self,
        catalog_id: RoomCatalogId,
        instance_id: AgentInstanceId,
    ) -> PortFuture<'_, DomainResult<RoomAllocation>>;
}
