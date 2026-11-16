//! 项目房间的认领和绑定（`specs/redesign/project-rooms.md` 的 3P-2b）：库里的两张表
//! （`repository_claim`、`room_repository_binding`），读 GitHub 的那个客户端，和认上以后替认领的人建房。

use agent_room_domain::{
    ids::{PrincipalId, RepositoryClaimId, RoomCatalogId},
    project_rooms::{ClaimCheckResult, GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
    time::UtcMillis,
};

use crate::{persistence::RepositoryResult, ports::PortFuture};

/// 一个等着核对的认领。认上就删，所以存着的都是在等的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryClaimRecord {
    pub id: RepositoryClaimId,
    pub principal_id: PrincipalId,
    pub repository: GithubRepository,
    /// 仓库里那一行带的码。绑已有房间时是那间房的分享码；没有时认上以后就是新房间的分享码。
    pub share_code: RoomShareCode,
    /// 要绑的已有房间；没有就是认上了才建。
    pub target: Option<RoomCatalogId>,
    pub claimed_at: UtcMillis,
    pub last_check: Option<RepositoryCheckRecord>,
    pub check_count: u32,
}

/// 上次核对的时间和结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryCheckRecord {
    pub at: UtcMillis,
    pub result: ClaimCheckResult,
}

/// 存认领的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryClaimInsertOutcome {
    /// 新存下的这一个。
    Inserted(RepositoryClaimRecord),
    /// 同一个人同一个仓库、要绑的房间也一样，已经在等了：回原来那个。
    Existing(RepositoryClaimRecord),
    /// 他已经有这么多个在等了。
    TooMany,
    /// 新生成的码被别的认领或房间占了，换一个再存。
    ShareCodeTaken,
}

/// 开始核对一个认领：先记下这一刻，同一个认领别处就不再同时核对。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryClaimCheckStart {
    /// 记下了，回记下以后的认领（`last_check` 是这一刻，结果还是上次的；第一次是 `checking`）。
    Started(RepositoryClaimRecord),
    /// 上次核对还没过够时间，这一刻以后才能再核对。
    TooSoon { retry_at: UtcMillis },
    /// 没有这个认领了（认上了、作废了、撤了）。
    Gone,
}

/// 一间房绑着的仓库。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryBindingRecord {
    pub catalog_id: RoomCatalogId,
    pub repository: GithubRepository,
    pub verified_by: PrincipalId,
    pub verified_at: UtcMillis,
    pub verified_file: RepositoryFile,
}

/// 绑着某个仓库的那间房，核对新的认领时要看的几样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundRepositoryRoom {
    pub binding: RepositoryBindingRecord,
    /// 那间房现在的分享码，仓库里原来那一行写的就是它。
    pub share_code: Option<RoomShareCode>,
    /// 平台冻结了它、或者从目录里拿掉了：照旧占着这个仓库，别人认领不了。
    pub held: bool,
}

/// 一间房能不能绑仓库要看的几样。几个开关是库里各自独立的几样事实，合不成一个状态。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProjectRoomCandidate {
    pub catalog_id: RoomCatalogId,
    pub owner: Option<PrincipalId>,
    /// 还在用，没归档、没关。
    pub active: bool,
    /// 是子房间：子房间不能单独公开，也不能绑仓库。
    pub sub_room: bool,
    /// 正公开着。
    pub listed: bool,
    pub share_code: Option<RoomShareCode>,
    /// 平台冻结了、从目录里拿掉了。
    pub held: bool,
    /// 已经绑着的仓库。
    pub bound: Option<GithubRepository>,
}

impl ProjectRoomCandidate {
    /// 这个人能不能拿这间房去认领：他是房主，房间公开着、还在用、不是子房间、平台没拿掉。
    #[must_use]
    pub fn bindable_by(&self, principal_id: PrincipalId) -> bool {
        self.owner == Some(principal_id)
            && self.active
            && !self.sub_room
            && self.listed
            && !self.held
            && self.share_code.is_some()
    }
}

/// 认上以后把仓库绑到一间已有的房间上，同一个事务里删掉认领；原来绑着别的房间时一起挪过来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindRepository {
    pub claim_id: RepositoryClaimId,
    pub catalog_id: RoomCatalogId,
    pub repository: GithubRepository,
    /// 认领的人：得还是这间房的房主。
    pub verified_by: PrincipalId,
    pub verified_at: UtcMillis,
    pub verified_file: RepositoryFile,
    /// 仓库里那一行的码：得还是这间房的分享码。
    pub share_code: RoomShareCode,
    /// 原来绑着这个仓库的房间：核对时它的链接已经不在仓库里了，绑定转过来。
    pub replacing: Option<RoomCatalogId>,
}

/// 绑上的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindRepositoryOutcome {
    Bound,
    /// 认领已经没了（别处认上了、撤了、作废了）。
    ClaimGone,
    /// 这间房不能绑了：不公开了、关了、换了房主、换了分享码、被平台拿掉了，或者已经绑着别的仓库。
    RoomNotEligible,
    /// 仓库刚被别的房间绑上了。
    RepositoryTaken,
}

/// 房主自己解绑的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnbindRepositoryOutcome {
    Unbound,
    NotBound,
    /// 他不是这间房的房主。
    NotOwner,
    /// 这间房公开着，他名下已经有一间别的不绑仓库的公开房间。
    LimitReached,
}

/// 后台这一轮该核对的认领：头 24 小时每 5 分钟一次，之后每小时一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueClaimQuery {
    pub now: UtcMillis,
    /// 认领早于这一刻的算“过了头 24 小时”。
    pub young_since: UtcMillis,
    /// 头 24 小时里上次核对早于这一刻的该核对了。
    pub young_checked_before: UtcMillis,
    /// 过了头 24 小时的，上次核对早于这一刻的该核对了。
    pub old_checked_before: UtcMillis,
    pub limit: u32,
}

pub trait ProjectRoomStore: Send + Sync {
    /// 存一个新认领。同一个人同一个仓库已经在等：要绑的房间也一样就回原来那个；不一样就换成新的
    /// （原来那个删掉，不多占名额）。他已经有 `max_pending` 个在等就不存。同一个事务里先锁住这个人，
    /// 两个请求同时来也不会超。码和别的认领、公开房间的码都不能重。
    fn insert_claim<'a>(
        &'a self,
        claim: &'a RepositoryClaimRecord,
        max_pending: u32,
    ) -> PortFuture<'a, RepositoryResult<RepositoryClaimInsertOutcome>>;

    fn claim(
        &self,
        claim_id: RepositoryClaimId,
    ) -> PortFuture<'_, RepositoryResult<Option<RepositoryClaimRecord>>>;

    /// 这个人在等的认领，最早的在前。
    fn claims_of(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RepositoryClaimRecord>>>;

    /// 撤掉他的这个认领。回答删没删。
    fn delete_claim(
        &self,
        claim_id: RepositoryClaimId,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<bool>>;

    /// 开始核对：上次核对早于 `checked_before`（或者从没核对过）才记下 `now`、次数加一。
    fn start_check(
        &self,
        claim_id: RepositoryClaimId,
        now: UtcMillis,
        checked_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RepositoryClaimCheckStart>>;

    /// 记下这次核对的结果。只在上次核对还是 `started_at` 那一次时记，别处后来又核对过就不动。
    fn finish_check(
        &self,
        claim_id: RepositoryClaimId,
        started_at: UtcMillis,
        result: ClaimCheckResult,
    ) -> PortFuture<'_, RepositoryResult<()>>;

    /// 后台这一轮该核对的认领，最久没核对的在前。
    fn due_claims(
        &self,
        query: DueClaimQuery,
    ) -> PortFuture<'_, RepositoryResult<Vec<RepositoryClaimId>>>;

    /// 删掉早于这一刻认领、还没认上的。回答删了几个。
    fn expire_claims(&self, claimed_before: UtcMillis) -> PortFuture<'_, RepositoryResult<u64>>;

    /// 一间房能不能绑仓库要看的几样。没有这间房（或者不是私人房间）是 `None`。
    fn project_room_candidate(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<ProjectRoomCandidate>>>;

    fn binding_for_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RepositoryBindingRecord>>>;

    /// 这个仓库（不论大小写）绑着的那间房。
    fn bound_room<'a>(
        &'a self,
        repository: &'a GithubRepository,
    ) -> PortFuture<'a, RepositoryResult<Option<BoundRepositoryRoom>>>;

    /// 认上以后绑到已有的房间上。先锁房间目录那一行再看房间还能不能绑（照转让的先后锁），同一个
    /// 事务里挪走原来的绑定、写下新的、删掉认领。
    fn bind_room<'a>(
        &'a self,
        request: &'a BindRepository,
    ) -> PortFuture<'a, RepositoryResult<BindRepositoryOutcome>>;

    /// 房主自己解绑：先锁房间目录那一行，再锁房主那一行，房间公开着时数他名下别的不绑仓库的公开房间。
    fn unbind_by_owner(
        &self,
        catalog_id: RoomCatalogId,
        owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<UnbindRepositoryOutcome>>;

    /// 房主再核对一次、找到了：记下谁、什么时候、在哪个文件里。回答这间房还绑没绑着这个仓库。
    fn refresh_binding<'a>(
        &'a self,
        binding: &'a RepositoryBindingRecord,
    ) -> PortFuture<'a, RepositoryResult<bool>>;
}

/// 读 GitHub 找那一行。实现只连 raw.githubusercontent.com，不跟跳转，有大小、时间和次数的上限。
pub trait RepositoryLinkReader: Send + Sync {
    /// 在仓库默认分支的 `AGENTS.md`、`README.md` 里找这几个码的链接（`<站点>/r/<码>`，后面不紧跟字母
    /// 或数字）。先读 `AGENTS.md`，码都找到了就不再读 README。没有这个文件不算出错。
    fn scan<'a>(
        &'a self,
        repository: &'a GithubRepository,
        codes: &'a [RoomShareCode],
    ) -> PortFuture<'a, Result<RepositoryScan, RepositoryReadFailure>>;

    /// 现在能不能读：全站这一分钟的次数用完了、或者 GitHub 限了次数正停着时，回什么时候能再读。
    fn next_read_at(&self) -> Option<UtcMillis>;
}

/// 读到的结果：每个码在哪个文件里找到的（找不到的不在里面）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryScan {
    pub found: Vec<(RoomShareCode, RepositoryFile)>,
    /// 有文件超过上限，只读了开头。
    pub truncated: bool,
}

impl RepositoryScan {
    /// 这个码在哪个文件里找到的。
    #[must_use]
    pub fn file_of(&self, code: &RoomShareCode) -> Option<RepositoryFile> {
        self.found
            .iter()
            .find(|(found, _)| found == code)
            .map(|(_, file)| *file)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryReadFailure {
    /// raw 回了跳转：仓库改过名、搬了家。
    Renamed,
    /// 读不了：超时、出错、被限了次数、这一分钟的次数用完了、地址不对。`retry_at` 是能再读的时候。
    Unavailable { retry_at: Option<UtcMillis> },
}

/// 认上以后替认领的人建项目房间：公开、不加密、直接进，名字是 `owner/repo`，分享码是认领的码，
/// 和绑定、删认领在同一个事务里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectRoom {
    pub claim_id: RepositoryClaimId,
    pub owner: PrincipalId,
    pub repository: GithubRepository,
    pub share_code: RoomShareCode,
    pub verified_at: UtcMillis,
    pub verified_file: RepositoryFile,
    pub replacing: Option<RoomCatalogId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRoomCreation {
    Created(RoomCatalogId),
    ClaimGone,
    RepositoryTaken,
    /// 没建成（聊天服务器不通、库里出错）。认领留着，下次再试。
    Failed,
}

pub trait ProjectRoomCreator: Send + Sync {
    fn create_project_room(
        &self,
        request: CreateProjectRoom,
    ) -> PortFuture<'_, ProjectRoomCreation>;
}

/// 建项目房间时库里那一步的结果（[`super::PrivateRoomListingStore::create_project_room`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRoomSaveOutcome {
    Saved,
    ClaimGone,
    RepositoryTaken,
    /// 认领的码被别的房间占了。码在仓库里写着，换不了。
    ShareCodeTaken,
    /// 这个房间号已经有了。
    Stale,
}
