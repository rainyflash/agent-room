//! 私人房间的 Agent 口令。接入分两步：先凭口令查看它对应的房间，CLI 与 MCP 据此选定在这个
//! 房间里用哪个人物；再让选定的人物凭口令加入，随后照常开会话进入这个房间。

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    rooms::MatrixRoomReference,
};

/// 口令对应的私人房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCodeRoom {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinCodeFailureKind {
    /// 格式不对：多半是抄错了，不计入猜错次数。
    InvalidCode,
    /// 口令不对，或房主已经停用、换了新口令。
    NotFound,
    /// 这个人物被移出过房间，要房主换一个新口令；或者这台设备不能带这个人物。
    Forbidden,
    /// 房间已归档，不再接纳 Agent。
    RoomUnavailable,
    /// 这台设备猜错的次数太多，要过一段时间再试。
    RateLimited,
    /// 服务端还不认识口令接口。
    Unsupported,
    NotAuthorized,
    ControlPlaneUnavailable,
    InvalidControlPlaneResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinCodeFailure {
    kind: JoinCodeFailureKind,
}

impl JoinCodeFailure {
    pub const fn new(kind: JoinCodeFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> JoinCodeFailureKind {
        self.kind
    }
}

pub type JoinCodeResult<T> = Result<T, JoinCodeFailure>;

pub trait ControlPlaneJoinCodeGateway: Send + Sync {
    /// 只查看口令对应的房间，不让任何 Agent 加入。
    fn resolve<'a>(&'a self, code: &'a str) -> PortFuture<'a, JoinCodeResult<JoinCodeRoom>>;

    /// 让这台设备的账号名下的这个 Agent 凭口令加入房间。
    fn redeem<'a>(
        &'a self,
        agent_id: AgentId,
        code: &'a str,
    ) -> PortFuture<'a, JoinCodeResult<JoinCodeRoom>>;
}
