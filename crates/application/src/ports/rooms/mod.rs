use std::collections::{BTreeMap, BTreeSet};

use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomInstance, RoomLanguage, RoomRegion, RoomReservation,
        RoomReservationState,
    },
    time::UtcMillis,
};

use crate::persistence::RepositoryResult;

use super::{MatrixResult, MatrixRoomId, MatrixTimelineEvent, PortFuture};

mod direct_sessions;
mod private_room_agents;
mod private_rooms;
mod provisioning;

use std::sync::Arc;

pub use direct_sessions::{
    DirectAgentProfile, DirectMatrixRoomCreation, DirectSessionAgentDirectory,
    DirectSessionMatrixProvisioner, DirectSessionMembershipGateway, DirectSessionRecord,
    DirectSessionStore,
};
pub use private_room_agents::{
    JoinCodeAttemptPolicy, PrivateRoomAgentAccessStore, PrivateRoomAgentKnockOutcome,
    PrivateRoomAgentKnockRecord, PrivateRoomAgentMemberLookup, PrivateRoomAgentMemberRecord,
    PrivateRoomJoinCodeRecord,
};
pub use private_rooms::{
    PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
    PrivateRoomAgentDirectory, PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner,
    PrivateRoomPrincipalDirectory, PrivateRoomSnapshot, PrivateRoomStore,
};
pub use provisioning::{
    RoomProvisioningClaim, RoomProvisioningClaimOutcome, RoomProvisioningFailureCode,
    RoomProvisioningGateway, RoomProvisioningJob, RoomProvisioningKind, RoomProvisioningStore,
    RoomProvisioningTarget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLobbyAccessRecord {
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub device_id: agent_room_domain::ids::DeviceId,
    /// Agent 此次代表的主体，即驱动该实例的设备所属账号。私人房间按它裁决入场资格。
    pub principal_id: agent_room_domain::ids::PrincipalId,
    pub matrix_user_id: super::MatrixUserId,
    pub active: bool,
}

pub trait AgentLobbyAccessRepository: Send + Sync {
    fn find_lobby_access(
        &self,
        agent_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentLobbyAccessRecord>>>;

    /// Resolve an active room in the requested public catalog. This grants no private-room access.
    fn find_public_lobby_room<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomInstanceId>>>;
}

/// 把受控 Matrix 用户绑定为最小房间成员能力，调用方不能选择任意认证方式。
pub trait AgentRoomMembershipFactory: Send + Sync {
    /// 绑定一个已由上层授权的 Matrix Agent 用户。
    ///
    /// # Errors
    ///
    /// 用户不属于受管命名空间或成员适配器配置无效时返回 Matrix 失败。
    fn bind(
        &self,
        matrix_user_id: &super::MatrixUserId,
    ) -> MatrixResult<Arc<dyn RoomMembershipGateway>>;
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicLobbyObservationRoom {
    pub catalog_id: RoomCatalogId,
    pub room_instance_id: RoomInstanceId,
    pub matrix_room_id: MatrixRoomReference,
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

    fn find_public_observation_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>>;
}

/// 不登录也能看公开大厅（specs/public-lobby-watch/design.md）：以建公开大厅的应用服务账号读
/// 大厅里最近的消息和当前状态。
///
/// 只给公开大厅用。实现用应用服务自己的账号读，不冒充任何用户；公开大厅不加密，读到的是明文。
pub trait PublicLobbyMatrixReader: Send + Sync {
    /// 最近最多 `limit` 条 Agent Room 消息与修订事件，旧的在前。
    fn recent_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        limit: u16,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>>;

    /// 房间此刻的全部状态事件。
    fn current_state<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>>;
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

/// 只暴露大厅加入 Saga 所需的 Matrix 成员能力。
pub trait RoomMembershipGateway: Send + Sync {
    fn join<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>>;

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>>;
}
