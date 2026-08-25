use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationAuditEvent, ModerationCase, ModerationRole, ModerationTarget,
    },
    time::{DurationMillis, UtcMillis},
};

use crate::persistence::RepositoryResult;

use super::{MatrixResult, MatrixRoomId, MatrixUserId, PortFuture};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationReportPolicy {
    pub maximum_reports: u16,
    pub window: DurationMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModerationReportSubmissionOutcome {
    Created(ModerationCase),
    Existing(ModerationCase),
    RateLimited { retry_at: UtcMillis },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModerationActionReservationOutcome {
    Reserved(ModerationAction),
    Existing(ModerationAction),
}

/// 举报、治理动作与追加审计的权威事务边界。
pub trait ModerationRepository: Send + Sync {
    /// 案件、最小证据与创建审计必须在同一事务中提交，并原子执行举报速率限制。
    fn submit_case<'a>(
        &'a self,
        case: &'a ModerationCase,
        audit: &'a ModerationAuditEvent,
        policy: ModerationReportPolicy,
    ) -> PortFuture<'a, RepositoryResult<ModerationReportSubmissionOutcome>>;

    fn find_case(
        &self,
        case_id: ModerationCaseId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationCase>>>;

    fn list_cases_for_reporter(
        &self,
        reporter_principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationCase>>>;

    /// 先持久化 pending 动作与 requested 审计，再允许应用层调用外部治理副作用。
    fn reserve_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>>;

    fn find_action(
        &self,
        action_id: ModerationActionId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationAction>>>;

    /// 动作终态与结果审计必须原子提交。
    fn finalize_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationAction>>;

    fn list_room_actions(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAction>>>;

    fn append_audit<'a>(
        &'a self,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    fn list_audit(
        &self,
        room_catalog_id: Option<RoomCatalogId>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAuditEvent>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationRoomContext {
    pub role: ModerationRole,
    pub matrix_room_id: MatrixRoomId,
    pub target_matrix_user_id: Option<MatrixUserId>,
}

/// 每次治理或审计读取前重新读取当前房间与平台权限。
pub trait ModerationAuthority: Send + Sync {
    fn may_report<'a>(
        &'a self,
        principal_id: PrincipalId,
        target: &'a ModerationTarget,
        room_catalog_id: Option<RoomCatalogId>,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    fn inspect_room<'a>(
        &'a self,
        principal_id: PrincipalId,
        room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>>;

    fn platform_role(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ModerationRole>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationEffectTarget {
    pub matrix_room_id: MatrixRoomId,
    pub target: ModerationTarget,
    pub target_matrix_user_id: Option<MatrixUserId>,
}

/// Matrix 只执行已经通过产品权限裁决并被持久化为 pending 的治理副作用。
pub trait ModerationEffectGateway: Send + Sync {
    fn apply<'a>(
        &'a self,
        action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>>;

    fn reverse<'a>(
        &'a self,
        action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>>;
}

pub trait ModerationIdentifierFactory: Send + Sync {
    fn moderation_case_id(&self) -> ModerationCaseId;
    fn moderation_action_id(&self) -> ModerationActionId;
    fn moderation_audit_event_id(&self) -> AuditEventId;
}
