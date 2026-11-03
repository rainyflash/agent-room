use agent_room_domain::{
    ids::{
        AgentId, AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId,
        RoomModerationGrantId,
    },
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditEvent, ModerationCase,
        ModerationRole, ModerationTarget,
    },
    room_moderation_grants::AgentModerationNote,
    rooms::RoomCatalogKind,
    time::{DurationMillis, UtcMillis},
};

use crate::persistence::RepositoryResult;

use super::{
    MatrixEventId, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture, RoomModerationGrant,
};

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
    /// 被管的人名下 Agent 的 Matrix 账号（他是主人或操作人、还没撤的）。禁言和撤销禁言跟着这个人一起
    /// 动它们，只动此刻在分片里的：Agent 的能力不能超过它代表的人。不是管人的动作时是空的。
    pub target_agent_matrix_user_ids: Vec<MatrixUserId>,
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
    /// 他名下 Agent 的 Matrix 账号，和 [`ModerationRoomContext::target_agent_matrix_user_ids`] 一样。
    pub target_agent_matrix_user_ids: Vec<MatrixUserId>,
    /// 撇开禁言，他此刻能不能说话：私人房间要受邀或者已经加入、权限里开着发言；他不是房间里的人时（比如
    /// 网络 Agent 的合成主体），看他名下凭口令或敲门进来的 Agent 成员开没开发言。公开大厅和私聊总是能。
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
    /// 被管的人名下 Agent 的 Matrix 账号。禁言和撤销禁言对此刻在这个分片里的跟着这个人一起做，别的
    /// 动作不看它。
    pub target_agent_matrix_user_ids: Vec<MatrixUserId>,
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

/// 私人房间里台账的踢出、封禁为什么做不了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationMemberRefusal {
    /// 对象是房主。
    Owner,
    /// 踢出时他已经不在房间里：没有受邀，也没有加入。
    NotInRoom,
    /// 房间不在了，或者已经归档。
    RoomUnavailable,
}

/// 私人房间里台账的踢出、封禁照成员一节那一套落，不只动聊天服务器（`specs/public-rooms/design.md`
/// “落到哪”）：只在聊天服务器上踢了、成员记录还是已加入的话，网页会把他当成还在房间里，又替他加入。
/// 台账已经核过动手的人管得了这个房间，这里以房主的身份改成员记录。成员记录和目录的失败也报成聊天
/// 服务器的失败（取最接近的种类）：对台账来说都是这一下没落成。
pub trait ModerationMemberGateway: Send + Sync {
    /// 落之前先看做不做得了，做不了交回为什么：不预留一条注定失败的动作。
    fn check(
        &self,
        kind: ModerationActionKind,
        room_catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationMemberRefusal>>>;

    /// 踢出就是移出，封禁就是封禁：改成员记录，在聊天服务器上踢出或封禁，把他名下在房间里的 Agent
    /// 移出。
    fn apply(
        &self,
        kind: ModerationActionKind,
        room_catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PortFuture<'_, MatrixResult<()>>;

    /// 撤回和到期解除：封禁就是解封，他回到“已移出”，之后能再邀请他或者他自己进来；踢出是照房间
    /// 新成员的默认权限重新邀请他。他已经不被封着、已经回到房间里了，什么也不做。
    fn reverse(
        &self,
        kind: ModerationActionKind,
        room_catalog_id: RoomCatalogId,
        target: PrincipalId,
    ) -> PortFuture<'_, MatrixResult<()>>;
}

/// Agent 在消息里看到的一个发言人认成的人（维护者 2026-11-03 定：动别人的 Agent 等于动它的主人）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationSubject {
    pub principal_id: PrincipalId,
    /// 认出来的是网络 Agent 替它记账的合成主体：它在房间里没有自己的账号，只有它的 Agent 账号。
    pub network_agent: bool,
}

/// 台账里 Agent 做的一条：是哪个 Agent、按哪份授权、它写的理由。执行人照旧是给授权的人。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationAttribution {
    pub action_id: ModerationActionId,
    pub agent_id: AgentId,
    pub grant_id: RoomModerationGrantId,
    pub note: AgentModerationNote,
}

/// 给房主和管理员看的一条，带着 Agent 此刻的名字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationRecord {
    pub attribution: AgentModerationAttribution,
    pub agent_display_name: String,
}

/// Agent 解封时撤回的一条封禁：`attribution.action_id` 是那条封禁，另记是哪个 Agent、按哪份授权、它写的
/// 理由，和解封请求自带的 ID（同一个请求再来一次时认出来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationReversal {
    pub request_id: ModerationActionId,
    pub attribution: AgentModerationAttribution,
}

/// 房间里已经落下、还没撤的一条封禁，和它是不是 Agent 做的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedBan {
    pub action: ModerationAction,
    pub by_agent: bool,
}

/// Agent 按管理授权动手：查它的授权、认人、和台账同一个事务记下是它做的。
pub trait AgentModerationStore: Send + Sync {
    /// 这个 Agent 在这个房间里此刻在用的授权（没撤、没到期）。
    fn active_agent_grant(
        &self,
        room_catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomModerationGrant>>>;

    /// 一个聊天账号是谁：人的账号就是他（账号还在用）；Agent 的账号认成它的主人，网络 Agent 认成替它记账的
    /// 合成主体。认不出来的为 `None`。
    fn moderation_subject<'a>(
        &'a self,
        matrix_user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationSubject>>>;

    /// 和 [`ModerationRepository::reserve_action`] 一样预留动作、写审计，同一个事务里再记下是哪个 Agent
    /// 按哪份授权做的。同一个动作已经在了就交回它，不再记。
    fn reserve_agent_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
        attribution: &'a AgentModerationAttribution,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>>;

    /// 这个房间里 Agent 做的动作，后做的在前。
    fn agent_actions(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentModerationRecord>>>;

    /// 这个房间里已经落下、还没撤的封禁，先做的在前，不限条数（台账的列表只给最近的 200 条）。
    fn applied_bans(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AppliedBan>>>;

    /// 和 [`ModerationRepository::finalize_action`] 一样把封禁记成撤回、写审计，同一个事务里再记下是哪个
    /// Agent 按哪份授权撤的。
    fn finalize_agent_reversal<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
        reversal: &'a AgentModerationReversal,
    ) -> PortFuture<'a, RepositoryResult<ModerationAction>>;

    /// 这个解封请求撤回的那条封禁；一次撤了几条的，交回最后撤的那条。
    fn agent_reversal(
        &self,
        request_id: ModerationActionId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentModerationReversal>>>;

    /// 这个房间里 Agent 撤回的动作，后撤的在前。
    fn agent_reversals(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentModerationRecord>>>;
}

pub trait ModerationIdentifierFactory: Send + Sync {
    fn moderation_case_id(&self) -> ModerationCaseId;
    fn moderation_action_id(&self) -> ModerationActionId;
    fn moderation_audit_event_id(&self) -> AuditEventId;
}
