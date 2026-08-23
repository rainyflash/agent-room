use agent_room_domain::{DomainResult, content::ContentObject, ids::ContentId};

use super::PortFuture;

pub trait ContentStore: Send + Sync {
    fn put<'a>(
        &'a self,
        content: &'a ContentObject,
        bytes: &'a [u8],
    ) -> PortFuture<'a, DomainResult<()>>;

    fn get(&self, id: ContentId) -> PortFuture<'_, DomainResult<Option<Vec<u8>>>>;
}
