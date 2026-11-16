//! 项目房间的认领（`specs/redesign/project-rooms.md` 的 3P-4a）：本机 Bridge 替这台电脑的账户经控制面
//! 认领一个 GitHub 仓库、现在核对。和建房一样用这台电脑的授权（设备签名）；网络 Agent 不能认领。
//!
//! 认领、核对都回同一份认领：还在等的带着认领 ID，认上了的带着绑上的那间房；那一行（`AGENTS.md`）和
//! README 徽章都是控制面照部署的站点拼好的，Bridge 原样转交。

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::{RepositoryClaimId, RoomCatalogId},
    project_rooms::GithubRepository,
    room_listing::RoomShareCode,
    time::UtcMillis,
};

/// 认领现在怎么样：还在等，或者认上了、绑在一间房上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryClaimStatus {
    Pending,
    Bound,
}

/// 上一次核对：什么时候、结果（控制面的说法，比如 `not_in_repository`、`repository_taken`）。
/// 结果原样转交，Bridge 不认它的取值：控制面以后多一种结果，已经装着的 Bridge 照样转得过去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryClaimCheck {
    pub at: UtcMillis,
    pub result: String,
}

/// 一个认领。认上以后控制面就删了认领：`claim_id` 是空的，`catalog_id` 是绑上的那间房。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryClaim {
    pub claim_id: Option<RepositoryClaimId>,
    pub repository: GithubRepository,
    pub share_code: RoomShareCode,
    pub share_url: String,
    /// 写进 `AGENTS.md` 的那一行。
    pub agents_line: String,
    /// README 徽章那一行（Markdown）。
    pub badge_markdown: String,
    pub status: RepositoryClaimStatus,
    /// 要绑的那间房；认上以后是绑上的那间房。认领了再建的、还在等的没有。
    pub catalog_id: Option<RoomCatalogId>,
    pub claimed_at: Option<UtcMillis>,
    pub last_check: Option<RepositoryClaimCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRoomFailureKind {
    /// 仓库要写成 `owner/repo`，名字照 GitHub 的规矩。
    InvalidRepository,
    /// 只有房主能拿这间房认领；或者这台电脑的账户不能认领。
    Forbidden,
    /// 没有这个认领（认上了、作废了、撤掉了），或者没有这间房。
    NotFound,
    /// 已经有 3 个认领在等。
    TooManyClaims,
    /// 仓库绑着一间平台冻结、拿掉的房间，现在认领不了。
    RepositoryTaken,
    /// 只能绑你名下公开着的房间；房间变了，认领作废。
    RoomNotEligible,
    /// 这间房已经绑着别的仓库。
    RoomAlreadyBound,
    /// 每个认领每分钟只能核对一次。
    CheckTooSoon,
    /// 这间房同时还有别人的认领在等。
    Conflict,
    /// 控制面还不认识认领的接口。
    Unsupported,
    NotAuthorized,
    ControlPlaneUnavailable,
    InvalidControlPlaneResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectRoomFailure {
    kind: ProjectRoomFailureKind,
}

impl ProjectRoomFailure {
    pub const fn new(kind: ProjectRoomFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> ProjectRoomFailureKind {
        self.kind
    }
}

pub type ProjectRoomResult<T> = Result<T, ProjectRoomFailure>;

pub trait ControlPlaneProjectRoomGateway: Send + Sync {
    /// 替这台电脑的账户认领仓库；带上 `target` 是绑那间已有的公开房间。同一个人同一个仓库再来，
    /// 控制面回同一个认领；仓库已经绑在他名下的房间上，回那间房。
    fn claim<'a>(
        &'a self,
        repository: &'a GithubRepository,
        target: Option<RoomCatalogId>,
    ) -> PortFuture<'a, ProjectRoomResult<RepositoryClaim>>;

    /// 现在核对一次（每个认领每分钟一次）。认上了回绑上的那间房。
    fn check(
        &self,
        claim_id: RepositoryClaimId,
    ) -> PortFuture<'_, ProjectRoomResult<RepositoryClaim>>;
}
