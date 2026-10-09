use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationAuditEvent, ModerationCase, ModerationRole, ModerationTarget,
    },
    rooms::RoomCatalogKind,
    time::{DurationMillis, UtcMillis},
};

use crate::persistence::RepositoryResult;

use super::{MatrixEventId, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture};

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

    fn list_room_cases(
        &self,
        room_catalog_id: RoomCatalogId,
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
    pub room_kind: RoomCatalogKind,
    /// 这个目录此刻全部活跃分片的 Matrix 房间，至少一个，最活跃的在前（和围观、登录的人看大厅选的
    /// 是同一个）。私人房间和私聊只有一个；公开大厅人多了会分成好几个分片。
    pub matrix_room_ids: Vec<MatrixRoomId>,
    pub target_matrix_user_id: Option<MatrixUserId>,
}

/// 翻到期动作时停在哪：上一页最后一个动作的到期时间和 ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationExpiryCursor {
    pub expires_at: UtcMillis,
    pub action_id: ModerationActionId,
}

/// 到期自动解除用的读取。这是系统自己做的，没有登录的人，也不看谁有权限，所以和
/// [`ModerationAuthority`] 分开。
pub trait ModerationExpiryRepository: Send + Sync {
    /// 已生效、到期时间不晚于 `now` 的动作，按到期时间和 ID 排，从 `after` 之后取最多 `limit` 个。
    fn list_due_actions(
        &self,
        now: UtcMillis,
        after: Option<ModerationExpiryCursor>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAction>>>;

    /// 同一个房间里、同一个对象上，还有没有别的同类动作在生效：已生效，没有期限或者 `now` 时还没到期。
    fn has_other_effective_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    /// 撤掉这个动作的副作用要落到哪：房间类别、此刻全部活跃分片（最活跃的在前，房间已经关了就是空的），
    /// 以及被管的人的 Matrix 账号（账号停用、删除了也照样给）。不看权限，`role` 总是
    /// [`ModerationRole::None`]。目录不存在时为 `None`。
    fn expiry_room<'a>(
        &'a self,
        action: &'a ModerationAction,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>>;
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

/// 治理落在哪一个 Matrix 房间（分片）上。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationEffectTarget {
    pub matrix_room_id: MatrixRoomId,
    /// 公开大厅谁都能说话、谁都能进，私人房间按成员的发言权管、只能受邀进：禁言和撤销按它来。
    pub room_kind: RoomCatalogKind,
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

    /// 这个房间里有没有这条事件（以建房间的应用服务账号读）。只读，用来找消息在哪个分片。
    fn contains_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<bool>>;
}

pub trait ModerationIdentifierFactory: Send + Sync {
    fn moderation_case_id(&self) -> ModerationCaseId;
    fn moderation_action_id(&self) -> ModerationActionId;
    fn moderation_audit_event_id(&self) -> AuditEventId;
}
