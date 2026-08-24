use std::collections::{BTreeMap, BTreeSet};

use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{
        RoomCatalog, RoomInstance, RoomLanguage, RoomRegion, RoomReservation, RoomReservationState,
    },
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomDirectoryQuery {
    pub language: Option<RoomLanguage>,
    pub region: Option<RoomRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicLobbyDirectoryEntry {
    pub catalog: RoomCatalog,
    pub active_instance_count: u16,
    pub online_agent_count: u32,
    pub activity_score_millis: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomAllocationMode {
    Automatic,
    Manual(RoomInstanceId),
}

/// 由受信任社交关系与邀请查询生成的分配证据。
///
/// 该结构不得直接从客户端 JSON 反序列化，否则用户可以伪造好友或邀请优先级。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomAllocationEvidence {
    pub previous_instance: Option<RoomInstanceId>,
    pub friends_per_instance: BTreeMap<RoomInstanceId, u16>,
    pub invited_instances: BTreeSet<RoomInstanceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomReservationClaim {
    pub reservation_id: RoomReservationId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub catalog_id: RoomCatalogId,
    pub mode: RoomAllocationMode,
    pub preferred_language: Option<RoomLanguage>,
    pub preferred_region: Option<RoomRegion>,
    pub evidence: RoomAllocationEvidence,
    pub reserved_at: UtcMillis,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomReservationOutcome {
    Reserved {
        reservation: RoomReservation,
        room: RoomInstance,
    },
    ExistingAssignment {
        reservation: RoomReservation,
        room: RoomInstance,
    },
    ProvisioningRequired {
        catalog: RoomCatalog,
    },
}

pub trait RoomDirectory: Send + Sync {
    fn list_public<'a>(
        &'a self,
        query: &'a RoomDirectoryQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>>;

    fn find_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>>;
}

/// 把候选查询、行锁、领域评分与槽位递增封装在同一短事务内。
pub trait RoomAllocationStore: Send + Sync {
    fn reserve<'a>(
        &'a self,
        claim: &'a RoomReservationClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomReservationOutcome>>;

    fn transition(
        &self,
        reservation_id: RoomReservationId,
        expected: RoomReservationState,
        target: RoomReservationState,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RoomReservation>>;

    fn expire_pending(&self, now: UtcMillis, limit: u16) -> PortFuture<'_, RepositoryResult<u16>>;
}
