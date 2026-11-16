//! 项目房间的认领和绑定（`specs/redesign/project-rooms.md` 的 3P-2b）。
//!
//! 认领的人拿到一个分享码，把带码的那一行（`AGENTS.md` 的一行或 README 的徽章）推到仓库的默认分支上。
//! 服务器去 GitHub 读到那一行就算认上：带着已有房间的就绑上那间房，没带的替他建一间项目房间再绑上。
//!
//! - 每个账户同时最多 3 个等着的认领；同一个人同一个仓库再来回同一个。
//! - 认领的人点“现在核对”每分钟一次；后台头 24 小时每 5 分钟一次、之后每小时一次，7 天没认上作废。
//! - 仓库已经绑着别的房间：那间房的链接还在仓库里（或者它被平台冻结、拿掉了）就一直等；不在了，地址
//!   转给新认领的这间，原来那间照旧公开、不再绑仓库。
//! - 房主能看绑着哪个仓库、再核对一次、解绑；解绑后名下有两间不绑仓库的公开房间就不让解。
//!
//! 开始核对之前先在库里记下这一刻（`start_check`），同一个认领别处就不再同时核对：多个控制面一起跑时
//! 也只读一遍 GitHub、只建一间房。

mod bindings;
mod checking;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_domain::{
    ids::{PrincipalId, RepositoryClaimId, RoomCatalogId},
    project_rooms::{ClaimCheckResult, GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
    time::UtcMillis,
};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, IdentifierFactory, PortFuture, ProjectRoomCreator, ProjectRoomStore,
        RepositoryBindingRecord, RepositoryCheckRecord, RepositoryClaimInsertOutcome,
        RepositoryClaimRecord, RepositoryLinkReader, SecretFactory,
    },
    private_rooms::{PrivateRoomActor, SHARE_CODE_ATTEMPTS, random_share_code},
};

/// 每个账户同时最多几个等着的认领，免得刷后台核对的次数。
pub const MAX_PENDING_CLAIMS: u32 = 3;
/// 认领的人自己点“现在核对”，每个认领多久一次。
pub const MANUAL_CHECK_INTERVAL_MILLIS: i64 = 60_000;
/// 认领多久没认上作废：7 天。
pub const CLAIM_LIFETIME_MILLIS: i64 = 7 * 24 * 60 * 60 * 1_000;
/// 头 24 小时后台每 5 分钟核对一次，之后每小时一次。
pub const YOUNG_CLAIM_MILLIS: i64 = 24 * 60 * 60 * 1_000;
pub const YOUNG_CHECK_INTERVAL_MILLIS: i64 = 5 * 60 * 1_000;
pub const OLD_CHECK_INTERVAL_MILLIS: i64 = 60 * 60 * 1_000;
/// 后台每一轮最多核对几个：每个最多读两次，全站每分钟最多 60 次。
pub const CLAIMS_PER_ROUND: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRoomFailureKind {
    /// 仓库不是 `owner/repo` 的样子，或者名字不合 GitHub 的规矩。
    InvalidRepository,
    InvalidRequest,
    /// 登录过期、网络 Agent 来认领，或者不是这间房的房主。
    Forbidden,
    /// 没有这个认领、这间房，或者这间房没绑仓库。
    NotFound,
    /// 已经有 3 个认领在等了。
    TooManyClaims,
    /// 仓库绑着一间被平台冻结、从目录里拿掉的房间，别人认领不了。
    RepositoryTaken,
    /// 拿来认领的房间不能绑：不公开、关了、是子房间、被平台拿掉了；或者认领期间房间变了，认领作废。
    RoomNotEligible,
    /// 拿来认领的房间已经绑着别的仓库。
    RoomAlreadyBound,
    /// 核对得太勤：`retry_at` 以后再来。
    CheckTooSoon,
    /// 解绑以后名下会有两间不绑仓库的公开房间。
    LimitReached,
    /// 同一间房同时有别人的认领在等（换过房主）。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectRoomFailure {
    operation: &'static str,
    kind: ProjectRoomFailureKind,
    retry_at: Option<UtcMillis>,
}

impl ProjectRoomFailure {
    pub const fn new(operation: &'static str, kind: ProjectRoomFailureKind) -> Self {
        Self {
            operation,
            kind,
            retry_at: None,
        }
    }

    #[must_use]
    pub const fn retrying_at(mut self, retry_at: UtcMillis) -> Self {
        self.retry_at = Some(retry_at);
        self
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> ProjectRoomFailureKind {
        self.kind
    }

    pub const fn retry_at(self) -> Option<UtcMillis> {
        self.retry_at
    }
}

pub type ProjectRoomResult<T> = Result<T, ProjectRoomFailure>;

/// 认领一个仓库。`repository` 是请求里原样的 `owner/repo`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRepository {
    pub actor: PrivateRoomActor,
    pub repository: String,
    /// 绑这间已有的公开房间；不给就是认上了再建。
    pub target: Option<RoomCatalogId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectClaim {
    pub actor: PrivateRoomActor,
    pub claim_id: RepositoryClaimId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectRoomRepository {
    pub actor: PrivateRoomActor,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStatus {
    Pending,
    Bound,
}

/// 给认领的人看的认领。认上以后认领就删了：`claim_id` 是空的，`catalog_id` 是绑上的那间房。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimView {
    pub claim_id: Option<RepositoryClaimId>,
    pub repository: GithubRepository,
    pub share_code: RoomShareCode,
    pub status: ClaimStatus,
    pub catalog_id: Option<RoomCatalogId>,
    pub claimed_at: Option<UtcMillis>,
    pub last_check: Option<RepositoryCheckRecord>,
}

impl ClaimView {
    fn pending(record: RepositoryClaimRecord) -> Self {
        Self {
            claim_id: Some(record.id),
            repository: record.repository,
            share_code: record.share_code,
            status: ClaimStatus::Pending,
            catalog_id: record.target,
            claimed_at: Some(record.claimed_at),
            last_check: record.last_check,
        }
    }

    fn bound(
        repository: GithubRepository,
        share_code: RoomShareCode,
        catalog_id: RoomCatalogId,
    ) -> Self {
        Self {
            claim_id: None,
            repository,
            share_code,
            status: ClaimStatus::Bound,
            catalog_id: Some(catalog_id),
            claimed_at: None,
            last_check: None,
        }
    }
}

/// 房主看的绑定：绑着哪个仓库、谁什么时候在哪个文件里核对上的，和这间房现在的分享码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingView {
    pub binding: RepositoryBindingRecord,
    pub share_code: Option<RoomShareCode>,
}

/// 房主再核对一次的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingCheckResult {
    /// 那一行还在，记下了这次是谁、在哪个文件里找到的。
    Found(RepositoryFile),
    /// 没找到。绑定照旧留着（认上以后不定期复查），只是照实说。只会是没找到、改过名、文件太大、
    /// GitHub 读不了这几种。
    Missing(ClaimCheckResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingCheckView {
    pub binding: BindingView,
    pub checked_at: UtcMillis,
    pub result: BindingCheckResult,
}

/// 后台一轮做了什么。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaimRound {
    pub expired: u64,
    pub checked: u32,
    pub bound: u32,
    /// 没认上、还在等的。
    pub pending: u32,
    /// 认领期间房间变了，认领作废的。
    pub dropped: u32,
    /// 这一轮的次数用完了、或者 GitHub 限了次数正停着，剩下的下一轮再来。
    pub deferred: bool,
}

pub trait ProjectRoomUseCases: Send + Sync {
    fn claim(&self, request: ClaimRepository) -> PortFuture<'_, ProjectRoomResult<ClaimView>>;

    fn inspect_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<ClaimView>>;

    /// 现在核对。认上了回绑上的那间房；没认上回认领，`last_check` 写着为什么。
    fn check_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<ClaimView>>;

    fn cancel_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<()>>;

    /// 房主看这间房绑着哪个仓库；没绑是 `None`。
    fn binding(
        &self,
        request: InspectRoomRepository,
    ) -> PortFuture<'_, ProjectRoomResult<Option<BindingView>>>;

    /// 房主解绑。没绑着也算成功。
    fn unbind(&self, request: InspectRoomRepository) -> PortFuture<'_, ProjectRoomResult<()>>;

    /// 房主再核对一次（每间房每分钟一次）。
    fn check_binding(
        &self,
        request: InspectRoomRepository,
    ) -> PortFuture<'_, ProjectRoomResult<BindingCheckView>>;
}

/// 后台核对：作废过期的，核对该核对的。
pub trait ProjectRoomClaimChecks: Send + Sync {
    fn check_due(&self) -> PortFuture<'_, Result<ClaimRound, RepositoryError>>;
}

pub struct ProjectRoomDependencies {
    pub store: Arc<dyn ProjectRoomStore>,
    pub reader: Arc<dyn RepositoryLinkReader>,
    pub creator: Arc<dyn ProjectRoomCreator>,
    pub secrets: Arc<dyn SecretFactory>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct ProjectRoomService {
    store: Arc<dyn ProjectRoomStore>,
    reader: Arc<dyn RepositoryLinkReader>,
    creator: Arc<dyn ProjectRoomCreator>,
    secrets: Arc<dyn SecretFactory>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
    /// 房主再核对一次的时间，每间房每分钟一次。只在这个进程里：绑定表不记核对的时间。
    binding_checks: Mutex<HashMap<RoomCatalogId, UtcMillis>>,
}

impl ProjectRoomService {
    pub fn new(dependencies: ProjectRoomDependencies) -> Self {
        Self {
            store: dependencies.store,
            reader: dependencies.reader,
            creator: dependencies.creator,
            secrets: dependencies.secrets,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            binding_checks: Mutex::new(HashMap::new()),
        }
    }

    async fn claim_internal(&self, request: ClaimRepository) -> ProjectRoomResult<ClaimView> {
        const OPERATION: &str = "project_room.claim";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        let repository = GithubRepository::parse(&request.repository).map_err(|_| {
            ProjectRoomFailure::new(OPERATION, ProjectRoomFailureKind::InvalidRepository)
        })?;
        if let Some(bound) = self
            .already_bound(principal_id, &repository, request.target, OPERATION)
            .await?
        {
            return Ok(bound);
        }
        let share_code = match request.target {
            Some(catalog_id) => {
                self.target_share_code(principal_id, catalog_id, &repository, OPERATION)
                    .await?
            }
            None => self.new_share_code(OPERATION)?,
        };
        let mut record = RepositoryClaimRecord {
            id: self.identifiers.repository_claim_id(),
            principal_id,
            repository,
            share_code,
            target: request.target,
            claimed_at: self.clock.now(),
            last_check: None,
            check_count: 0,
        };
        for _ in 0..SHARE_CODE_ATTEMPTS {
            let outcome = self
                .store
                .insert_claim(&record, MAX_PENDING_CLAIMS)
                .await
                .map_err(|error| repository_failure(OPERATION, &error))?;
            match outcome {
                RepositoryClaimInsertOutcome::Inserted(saved)
                | RepositoryClaimInsertOutcome::Existing(saved) => {
                    return Ok(ClaimView::pending(saved));
                }
                RepositoryClaimInsertOutcome::TooMany => {
                    return Err(ProjectRoomFailure::new(
                        OPERATION,
                        ProjectRoomFailureKind::TooManyClaims,
                    ));
                }
                // 新生成的码撞了换一个；已有房间的码撞了，是这间房同时有别人的认领在等。
                RepositoryClaimInsertOutcome::ShareCodeTaken if record.target.is_none() => {
                    record.share_code = self.new_share_code(OPERATION)?;
                }
                RepositoryClaimInsertOutcome::ShareCodeTaken => {
                    return Err(ProjectRoomFailure::new(
                        OPERATION,
                        ProjectRoomFailureKind::Conflict,
                    ));
                }
            }
        }
        Err(ProjectRoomFailure::new(
            OPERATION,
            ProjectRoomFailureKind::Internal,
        ))
    }

    /// 仓库已经绑着他自己要的那间房：直接回那间房。他没说要哪间、绑着的就是他名下的，也一样。
    /// 绑着一间被平台冻结、拿掉的房间：别人认领不了。
    async fn already_bound(
        &self,
        principal_id: PrincipalId,
        repository: &GithubRepository,
        target: Option<RoomCatalogId>,
        operation: &'static str,
    ) -> ProjectRoomResult<Option<ClaimView>> {
        let Some(bound) = self
            .store
            .bound_room(repository)
            .await
            .map_err(|error| repository_failure(operation, &error))?
        else {
            return Ok(None);
        };
        let catalog_id = bound.binding.catalog_id;
        if target.is_none_or(|target| target == catalog_id) {
            let candidate = self
                .store
                .project_room_candidate(catalog_id)
                .await
                .map_err(|error| repository_failure(operation, &error))?;
            if let (Some(candidate), Some(share_code)) = (candidate, bound.share_code.clone())
                && candidate.owner == Some(principal_id)
            {
                return Ok(Some(ClaimView::bound(
                    bound.binding.repository,
                    share_code,
                    catalog_id,
                )));
            }
        }
        if bound.held {
            return Err(ProjectRoomFailure::new(
                operation,
                ProjectRoomFailureKind::RepositoryTaken,
            ));
        }
        Ok(None)
    }

    /// 绑已有房间的认领：房间得是他名下公开着、还在用的主房间，没绑别的仓库。码就是房间现在的分享码。
    async fn target_share_code(
        &self,
        principal_id: PrincipalId,
        catalog_id: RoomCatalogId,
        repository: &GithubRepository,
        operation: &'static str,
    ) -> ProjectRoomResult<RoomShareCode> {
        let candidate = self
            .store
            .project_room_candidate(catalog_id)
            .await
            .map_err(|error| repository_failure(operation, &error))?
            .ok_or_else(|| ProjectRoomFailure::new(operation, ProjectRoomFailureKind::NotFound))?;
        if candidate.owner != Some(principal_id) {
            return Err(ProjectRoomFailure::new(
                operation,
                ProjectRoomFailureKind::Forbidden,
            ));
        }
        if candidate
            .bound
            .as_ref()
            .is_some_and(|bound| !bound.same_as(repository))
        {
            return Err(ProjectRoomFailure::new(
                operation,
                ProjectRoomFailureKind::RoomAlreadyBound,
            ));
        }
        if !candidate.bindable_by(principal_id) {
            return Err(ProjectRoomFailure::new(
                operation,
                ProjectRoomFailureKind::RoomNotEligible,
            ));
        }
        candidate.share_code.ok_or_else(|| {
            ProjectRoomFailure::new(operation, ProjectRoomFailureKind::RoomNotEligible)
        })
    }

    async fn inspect_claim_internal(&self, request: InspectClaim) -> ProjectRoomResult<ClaimView> {
        const OPERATION: &str = "project_room.claim.inspect";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        let record = self
            .own_claim(principal_id, request.claim_id, OPERATION)
            .await?;
        Ok(ClaimView::pending(record))
    }

    async fn cancel_claim_internal(&self, request: InspectClaim) -> ProjectRoomResult<()> {
        const OPERATION: &str = "project_room.claim.cancel";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        let deleted = self
            .store
            .delete_claim(request.claim_id, principal_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        if deleted {
            Ok(())
        } else {
            Err(ProjectRoomFailure::new(
                OPERATION,
                ProjectRoomFailureKind::NotFound,
            ))
        }
    }

    async fn own_claim(
        &self,
        principal_id: PrincipalId,
        claim_id: RepositoryClaimId,
        operation: &'static str,
    ) -> ProjectRoomResult<RepositoryClaimRecord> {
        self.store
            .claim(claim_id)
            .await
            .map_err(|error| repository_failure(operation, &error))?
            .filter(|record| record.principal_id == principal_id)
            .ok_or_else(|| ProjectRoomFailure::new(operation, ProjectRoomFailureKind::NotFound))
    }

    /// 认领的人：登录还有效。网络 Agent 不能认领，它不在仓库目录里，也不替谁的账户做事。
    fn claimant(
        &self,
        actor: &PrivateRoomActor,
        operation: &'static str,
    ) -> ProjectRoomResult<PrincipalId> {
        if matches!(actor, PrivateRoomActor::NetworkAgent(_))
            || self.clock.now() >= actor.expires_at()
        {
            return Err(ProjectRoomFailure::new(
                operation,
                ProjectRoomFailureKind::Forbidden,
            ));
        }
        Ok(actor.principal_id())
    }

    fn new_share_code(&self, operation: &'static str) -> ProjectRoomResult<RoomShareCode> {
        random_share_code(self.secrets.as_ref()).ok_or_else(|| {
            ProjectRoomFailure::new(operation, ProjectRoomFailureKind::DependencyUnavailable)
        })
    }
}

impl ProjectRoomUseCases for ProjectRoomService {
    fn claim(&self, request: ClaimRepository) -> PortFuture<'_, ProjectRoomResult<ClaimView>> {
        Box::pin(self.claim_internal(request))
    }

    fn inspect_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<ClaimView>> {
        Box::pin(self.inspect_claim_internal(request))
    }

    fn check_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<ClaimView>> {
        Box::pin(self.check_claim_internal(request))
    }

    fn cancel_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<()>> {
        Box::pin(self.cancel_claim_internal(request))
    }

    fn binding(
        &self,
        request: InspectRoomRepository,
    ) -> PortFuture<'_, ProjectRoomResult<Option<BindingView>>> {
        Box::pin(self.binding_internal(request))
    }

    fn unbind(&self, request: InspectRoomRepository) -> PortFuture<'_, ProjectRoomResult<()>> {
        Box::pin(self.unbind_internal(request))
    }

    fn check_binding(
        &self,
        request: InspectRoomRepository,
    ) -> PortFuture<'_, ProjectRoomResult<BindingCheckView>> {
        Box::pin(self.check_binding_internal(request))
    }
}

impl ProjectRoomClaimChecks for ProjectRoomService {
    fn check_due(&self) -> PortFuture<'_, Result<ClaimRound, RepositoryError>> {
        Box::pin(self.check_due_internal())
    }
}

fn repository_failure(operation: &'static str, error: &RepositoryError) -> ProjectRoomFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::NotFound => ProjectRoomFailureKind::NotFound,
        RepositoryErrorKind::Conflict => ProjectRoomFailureKind::Conflict,
        RepositoryErrorKind::Forbidden => ProjectRoomFailureKind::Forbidden,
        RepositoryErrorKind::Unavailable => ProjectRoomFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            ProjectRoomFailureKind::Internal
        }
    };
    ProjectRoomFailure::new(operation, kind)
}

/// `at` 再往前 `millis` 毫秒，算不出来就是最早的那一刻。
fn before(at: UtcMillis, millis: i64) -> UtcMillis {
    UtcMillis::new(at.value().saturating_sub(millis)).unwrap_or(at)
}

/// `at` 再往后 `millis` 毫秒。
fn after(at: UtcMillis, millis: i64) -> UtcMillis {
    UtcMillis::new(at.value().saturating_add(millis)).unwrap_or(at)
}
