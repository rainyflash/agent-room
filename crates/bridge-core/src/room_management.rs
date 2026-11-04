//! Agent 自己建房间、管房间（`specs/agent-rooms/design.md`）：本机 Bridge 替这台电脑的账号，经控制面
//! 建私人房间、管口令和敲门、邀请人、移出 Agent。Agent 只管 Agent 建的房间，由控制面把关。
//!
//! 主人给了管理授权的房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”），Agent 也经这里读规则、
//! 照规则禁言、踢出、封禁、解开它封的；能不能做由控制面按授权把关。

use std::fmt;

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, PrincipalId, RoomCatalogId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};

/// 建好的房间：拿房间 ID 照常进去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedRoom {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub name: String,
}

/// 房间的口令，给人看的形式 `XXXX-XXXX-XXXX`。调试输出里不出现口令本身。
#[derive(Clone, PartialEq, Eq)]
pub struct RoomJoinCode {
    pub code: String,
    pub created_at: UtcMillis,
}

impl fmt::Debug for RoomJoinCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomJoinCode")
            .field("code", &"<redacted>")
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// 房间现在的口令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomCodeState {
    /// 没有口令。
    Missing,
    /// 有口令，但看不到了：以前生成的，或者服务器没法保管口令。要给 Agent 就换一个。
    Hidden,
    Visible(RoomJoinCode),
}

/// 在等、没作废的敲门，先敲的在前。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomKnock {
    pub agent_id: AgentId,
    pub display_name: String,
    pub knocked_at: UtcMillis,
    pub expires_at: UtcMillis,
}

/// 主人在这个房间给 Agent 的管理授权：照什么规则、能做哪几样、到什么时候。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomModerationGrant {
    pub rules: String,
    /// `mute`、`kick`、`ban`、`unban`。
    pub actions: Vec<String>,
    pub expires_at: UtcMillis,
}

/// Agent 照授权动手。对象是发言人的聊天账号；理由会写进台账给房主看。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomModerationRequest {
    /// `mute`、`kick`、`ban`、`unban`。
    pub action: &'static str,
    pub target: String,
    /// 禁言、踢出、封禁的原因；解封没有。
    pub reason: Option<String>,
    pub note: String,
    /// 只有禁言有；没有就一直禁着。
    pub duration_seconds: Option<u64>,
}

/// 台账里记下的这一条（解封是它撤回的那条封禁）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomModerationOutcome {
    pub action_id: String,
    pub action: String,
    pub status: String,
    pub expires_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomManagementFailureKind {
    /// 请求不对，例如名字不合规、账户 ID 不对。
    InvalidRequest,
    /// 不是 Agent 建的房间，或者这个账号在房间里没有管理权限。
    Forbidden,
    /// 房间、敲门或 Agent 不在。
    NotFound,
    /// 同一个房间 ID 已经建了别的房间，或者房间已经关了。
    Conflict,
    RateLimited,
    /// 下面几种只有照管理授权动手时才有：没有授权、授权里没有这一样、对象是房主或管理员（或者是
    /// Agent 自己、它的主人）、找不到对象、对象不在房间里、对象没被封着、封着他的是人。
    NotGranted,
    NotAllowed,
    Protected,
    TargetNotFound,
    NotInRoom,
    NotBanned,
    BannedByPerson,
    /// 控制面还没有这些接口。
    Unsupported,
    NotAuthorized,
    ControlPlaneUnavailable,
    InvalidControlPlaneResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomManagementFailure {
    kind: RoomManagementFailureKind,
}

impl RoomManagementFailure {
    pub const fn new(kind: RoomManagementFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> RoomManagementFailureKind {
        self.kind
    }
}

pub type RoomManagementResult<T> = Result<T, RoomManagementFailure>;

pub trait ControlPlaneRoomManagementGateway: Send + Sync {
    /// 替账号建私人房间，记下是哪个 Agent 建的；房主照旧是这个账号。同一个 `catalog_id` 重试
    /// 拿回同一个房间。
    fn create<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        name: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, RoomManagementResult<CreatedRoom>>;

    /// 房间现在的口令。只看，不生成。
    fn code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<RoomCodeState>>;

    /// 生成口令；已经有的换成新的，旧的马上失效。
    fn generate_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<RoomJoinCode>>;

    fn disable_code(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RoomManagementResult<()>>;

    fn knocks(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<Vec<RoomKnock>>>;

    /// 让在敲门的 Agent 进来（控制面替它进房间），或者不让进。
    fn answer_knock(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        admit: bool,
    ) -> PortFuture<'_, RoomManagementResult<()>>;

    /// 按账户 ID 邀请一个人，给查看、发言和自动发言。
    fn invite(
        &self,
        catalog_id: RoomCatalogId,
        account_id: PrincipalId,
    ) -> PortFuture<'_, RoomManagementResult<()>>;

    /// 移出凭口令或敲门进来的 Agent。
    fn remove_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RoomManagementResult<()>>;

    /// 主人在这个房间给这个 Agent 的管理授权；没有（或撤了、到期了）是 `None`。
    fn moderation_grant(
        &self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<Option<RoomModerationGrant>>>;

    /// 照授权动手。`action_id` 是新生成的动作 ID（UUID v7），控制面拿它当幂等键。
    fn moderate<'a>(
        &'a self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
        action_id: uuid::Uuid,
        request: &'a RoomModerationRequest,
    ) -> PortFuture<'a, RoomManagementResult<RoomModerationOutcome>>;
}
