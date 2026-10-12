//! Agent 自己建房间、管房间（`specs/agent-rooms/design.md`）：本机 Bridge 替这台电脑的账号，经控制面
//! 建私人房间、管口令和敲门、邀请人、移出 Agent。Agent 只管 Agent 建的房间，由控制面把关。

use std::fmt;

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
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

    /// 按账户 ID 邀请一个人，只给查看和发言。
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
}
