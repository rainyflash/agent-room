use std::{collections::BTreeSet, num::NonZeroU16};

use crate::{
    DomainError, DomainResult,
    ids::{AgentInstanceId, RoomCatalogId, RoomInstanceId},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCatalog {
    id: RoomCatalogId,
    capacity_per_instance: NonZeroU16,
}

impl RoomCatalog {
    pub const fn new(id: RoomCatalogId, capacity_per_instance: NonZeroU16) -> Self {
        Self {
            id,
            capacity_per_instance,
        }
    }

    pub const fn id(&self) -> RoomCatalogId {
        self.id
    }

    pub const fn capacity_per_instance(&self) -> NonZeroU16 {
        self.capacity_per_instance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    Joined,
    AlreadyPresent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveOutcome {
    Left,
    WasAbsent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAllocation {
    id: RoomInstanceId,
    catalog_id: RoomCatalogId,
    capacity: NonZeroU16,
    occupants: BTreeSet<AgentInstanceId>,
}

impl RoomAllocation {
    pub const fn new(id: RoomInstanceId, catalog_id: RoomCatalogId, capacity: NonZeroU16) -> Self {
        Self {
            id,
            catalog_id,
            capacity,
            occupants: BTreeSet::new(),
        }
    }

    pub const fn id(&self) -> RoomInstanceId {
        self.id
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub fn occupant_count(&self) -> usize {
        self.occupants.len()
    }

    /// 幂等加入房间分配。
    ///
    /// # Errors
    ///
    /// 新实例加入会突破房间容量时返回容量错误。
    pub fn join(&mut self, instance_id: AgentInstanceId) -> DomainResult<JoinOutcome> {
        if self.occupants.contains(&instance_id) {
            return Ok(JoinOutcome::AlreadyPresent);
        }

        if self.occupants.len() >= usize::from(self.capacity.get()) {
            return Err(DomainError::CapacityExceeded {
                capacity: self.capacity.get(),
            });
        }

        self.occupants.insert(instance_id);
        Ok(JoinOutcome::Joined)
    }

    pub fn leave(&mut self, instance_id: AgentInstanceId) -> LeaveOutcome {
        if self.occupants.remove(&instance_id) {
            LeaveOutcome::Left
        } else {
            LeaveOutcome::WasAbsent
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use proptest::prelude::*;
    use uuid::Uuid;

    use super::RoomAllocation;
    use crate::ids::{AgentInstanceId, RoomCatalogId, RoomInstanceId};

    proptest! {
        #[test]
        fn 任意加入序列都不能突破容量(capacity in 1_u16..64, ids in prop::collection::vec(any::<u128>(), 0..256)) {
            let capacity = NonZeroU16::new(capacity).expect("生成容量始终非零");
            let mut allocation = RoomAllocation::new(
                RoomInstanceId::from_uuid(Uuid::from_u128(1)),
                RoomCatalogId::from_uuid(Uuid::from_u128(2)),
                capacity,
            );

            for id in ids {
                let _ = allocation.join(AgentInstanceId::from_uuid(Uuid::from_u128(id)));
                prop_assert!(allocation.occupant_count() <= usize::from(capacity.get()));
            }
        }
    }
}
