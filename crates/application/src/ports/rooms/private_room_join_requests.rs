//! 申请进公开房间（`specs/public-rooms/design.md`）。申请放在私人房间聚合之外：批准时照旧经
//! 私人房间服务以房主的名义邀请，申请本身只记谁、留了什么话、处理到哪一步。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_join_requests::{JoinRequestMessage, JoinRequestStatus},
    time::UtcMillis,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequestRecord {
    pub catalog_id: RoomCatalogId,
    pub requester: PrincipalId,
    pub message: JoinRequestMessage,
    pub status: JoinRequestStatus,
    pub requested_at: UtcMillis,
    pub decided_at: Option<UtcMillis>,
    /// 批准或拒绝的人；作废、撤回的没有。
    pub decided_by: Option<PrincipalId>,
}

/// 给房主和管理员看的在等的申请，带着申请人此刻的昵称。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingJoinRequest {
    pub record: JoinRequestRecord,
    pub requester_display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinRequestCreateOutcome {
    Created,
    /// 他在这间房已经有一个在等的申请，原样回答那一个。
    AlreadyPending(JoinRequestRecord),
}

/// 一轮清理做了多少。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JoinRequestSweep {
    /// 房间不公开了、归档了，还在等的作废了几个。
    pub voided: u64,
    /// 处理完超过保留期、删掉了几个。
    pub deleted: u64,
}

pub trait PrivateRoomJoinRequestStore: Send + Sync {
    /// 他在这间房在等的申请。
    fn pending_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<JoinRequestRecord>>>;

    /// 他在这间房最近一次被拒的时间。
    fn last_rejected_at(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<UtcMillis>>>;

    /// 记下一个在等的申请。同一个人在这间房已经有一个在等的就不再记。
    fn create_join_request<'a>(
        &'a self,
        record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<JoinRequestCreateOutcome>>;

    /// 这间房在等的申请，先来的在前。
    fn pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PendingJoinRequest>>>;

    /// 把他在这间房在等的申请改成 `status`（批准、拒绝、撤回）。没有在等的回 `false`。
    fn decide_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
        status: JoinRequestStatus,
        decided_by: Option<PrincipalId>,
        decided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>>;

    /// 这间房在等的申请全部作废（改回私密时），回答作废了几个。
    fn void_pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
        voided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>>;

    /// 定时清理：房间不公开了、归档了的作废；处理时间早于 `decided_before` 的删掉。
    fn sweep_join_requests(
        &self,
        now: UtcMillis,
        decided_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<JoinRequestSweep>>;
}
