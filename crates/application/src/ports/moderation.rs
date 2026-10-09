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

/// 领到的一个到期动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationExpiryClaim {
    pub action: ModerationAction,
    /// 这是第几次领到它（“下一轮再看”的那几次排下次时退回去，不算）。排下次时拿它认是不是还是这一次
    /// 领的，退避也按它算。
    pub attempt: u32,
    /// 之前撤不成时记下的失败码。只有第一次失败写审计。
    pub previous_failure_code: Option<String>,
}

/// 领到以后没记成到期，下次什么时候再领。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationExpiryReschedule {
    /// 撤不成：`at` 以后再领，记下失败码（和落治理失败是同一套 `matrix.*`）。
    Retry {
        at: UtcMillis,
        failure_code: &'static str,
    },
    /// 这次定不下来（有一条禁言正在落、动完再看一眼变了、别处正在处理同一个人）：`at` 以后再领。
    /// 不算失败，领过的次数退回去。
    Defer { at: UtcMillis },
}

/// 到期自动解除用的读取。这是系统自己做的，没有登录的人，也不看谁有权限，所以和
/// [`ModerationAuthority`] 分开。
pub trait ModerationExpiryRepository: Send + Sync {
    /// 领一个到期的动作：已生效、到期时间不晚于 `now`、下次能领的时间空着或者已经过了的里面，先到期的
    /// 先领。领到的同时把下次能领的时间设成 `lease_until`（租约：做到一半进程断了，过了这个时间别的
    /// 实例接着做），领过的次数加一。别的实例正在领的跳过；没有能领的交回 `None`。
    fn claim_due_action(
        &self,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationExpiryClaim>>>;

    /// 领到以后没记成到期，按 `reschedule` 排下次。只在还是已生效、领过的次数还是领到时那个数时改，
    /// 交回改了没有：租约过了、别的实例又领走了，前一个就改不动。
    fn reschedule_expiry<'a>(
        &'a self,
        claim: &'a ModerationExpiryClaim,
        reschedule: ModerationExpiryReschedule,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    /// 同一个房间里、同一个对象上，还有没有别的同类动作在生效：已生效，没有期限或者 `now` 时还没到期。
    /// 管人的动作按 UUID 认人，写法不一样（大写、不带横线）也是同一个人。
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

/// 一个人在一个房间里此刻的禁言情况。禁言到期解除和撤回禁言都按它判断这个人此刻该不该禁着。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationMuteStanding {
    pub room_kind: RoomCatalogKind,
    /// 此刻全部活跃分片的 Matrix 房间，最活跃的在前；房间关了就是空的。
    pub matrix_room_ids: Vec<MatrixRoomId>,
    /// 这个人的 Matrix 账号，账号停用了也照样给；查不到这个人时为 `None`。
    pub target_matrix_user_id: Option<MatrixUserId>,
    /// 撇开禁言，他此刻能不能说话：私人房间要受邀或者已经加入、权限里开着发言；公开大厅和私聊总是能。
    pub may_speak: bool,
    /// 他在这个房间里已经落下（`applied`）和正在落（`pending`）的禁言，按 UUID 认人，先做的在前。
    pub mutes: Vec<ModerationAction>,
}

/// 禁言到期解除和撤回禁言用：读一个人此刻的禁言情况；同一个房间里同一个人，一次只做一个。
pub trait ModerationMuteLedger: Send + Sync {
    /// 这个人在这个房间里此刻的禁言情况。目录不存在时为 `None`。
    fn mute_standing<'a>(
        &'a self,
        room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationMuteStanding>>>;

    /// 拿这个房间里这个人的禁言锁，按 UUID 认人。别处正拿着时交回 `None`，不等。放掉以前别处拿不到；
    /// 进程断了，锁也跟着放掉。
    fn try_lock_mutes<'a>(
        &'a self,
        room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<Box<dyn ModerationMuteLock>>>>;
}

/// 拿着的禁言锁。丢掉不放也会放掉，只是晚一点。
pub trait ModerationMuteLock: Send {
    /// 放掉。放不掉也不要紧：连接一断，锁就放了。
    fn release(self: Box<Self>) -> PortFuture<'static, ()>;
}

/// 新开的公开大厅分片开始接人之前要补上的一条治理：此刻仍生效的禁言或封禁，和被管的人的 Matrix
/// 账号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingModeration {
    pub action: ModerationAction,
    pub target_matrix_user_id: MatrixUserId,
}

/// 新开的公开大厅分片要补上哪些治理。禁言、封禁管的是人，一直生效到撤销或到期；踢出是一次性的，
/// 隐藏只管消息所在的那个分片，都不用补。
pub trait StandingModerationSource: Send + Sync {
    /// 这个目录此刻仍生效的禁言和封禁（已经落下、没撤销、没到期），先做的在前。
    fn standing_person_actions(
        &self,
        room_catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<StandingModeration>>>;
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
