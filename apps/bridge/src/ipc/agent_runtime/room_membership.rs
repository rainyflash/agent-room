//! 会话的 Agent 再进一个房间、离开一个房间（`specs/agent-identity/design.md` 第 3 步）。进房间和
//! 开会话时同一套：这台设备签名找控制面，服务器替它在 Matrix 上加入；名片由之后的同步补写。

use std::{sync::Arc, time::Duration};

use agent_room_application::ports::MatrixRoomId;
use agent_room_bridge_core::lobby_session::{
    AgentLobbySessionConfig, AgentLobbySessionFailure, AgentLobbySessionFailureKind,
    AgentLobbySessionService, ControlPlaneLobbyEntryOutcome,
};
use agent_room_bridge_ipc::{
    IpcEnterRoomRequest, IpcErrorCategory, IpcLeaveRoomRequest, IpcResponse, IpcSessionRoom,
};
use agent_room_domain::{ids::RoomCatalogId, rooms::MatrixRoomReference};
use uuid::{Uuid, Version};

use super::{AgentRuntimeIpcFacade, BridgeIpcDispatchFailure, invalid_request};
use crate::{ipc::agent_runtime_unavailable, joined_rooms::JoinedRooms};

/// 等这台电脑上的 Matrix 客户端同步到刚进的房间，最多这么久。长轮询一看到加入就返回，一般一两秒。
const LOCAL_JOIN_WAIT: Duration = Duration::from_secs(20);

/// 会话的 Agent 进出房间用的：开会话时那套大厅服务和偏好（语言、地区）、开会话时进的房间，
/// 和同步里看到的房间。
pub(crate) struct AgentRoomEntry {
    lobby: Arc<AgentLobbySessionService>,
    preferences: AgentLobbySessionConfig,
    session_room: RoomCatalogId,
    joined: Arc<JoinedRooms>,
}

impl AgentRoomEntry {
    pub(crate) const fn new(
        lobby: Arc<AgentLobbySessionService>,
        preferences: AgentLobbySessionConfig,
        session_room: RoomCatalogId,
        joined: Arc<JoinedRooms>,
    ) -> Self {
        Self {
            lobby,
            preferences,
            session_room,
            joined,
        }
    }

    fn config(
        &self,
        catalog_id: RoomCatalogId,
        room: Option<MatrixRoomReference>,
    ) -> AgentLobbySessionConfig {
        AgentLobbySessionConfig::new(
            catalog_id,
            self.preferences.preferred_language().cloned(),
            self.preferences.preferred_region().cloned(),
        )
        .with_target_room(room)
    }
}

impl AgentRuntimeIpcFacade {
    /// 再进一个房间，不离开已经在的。进过的再进一次照样回答这个房间。
    pub(in crate::ipc) async fn enter_room(
        &self,
        request: IpcEnterRoomRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let entry = runtime
            .room_entry
            .clone()
            .ok_or_else(agent_runtime_unavailable)?;
        let catalog_id = parse_catalog_id(&request.room.catalog_id)?;
        let target = request
            .room
            .room_id
            .map(MatrixRoomReference::new)
            .transpose()
            .map_err(|_| invalid_request("bridge.ipc.room_invalid"))?;
        let outcome = entry
            .lobby
            .enter(&runtime.identity, &entry.config(catalog_id, target))
            .await
            .map_err(map_lobby_failure)?;
        let room = match outcome {
            ControlPlaneLobbyEntryOutcome::Joined(room) => room,
            ControlPlaneLobbyEntryOutcome::ProvisioningBusy { .. } => {
                return Err(retryable("bridge.lobby_provisioning_busy"));
            }
            ControlPlaneLobbyEntryOutcome::CapacityChanged { .. } => {
                return Err(retryable("bridge.lobby_capacity_changed"));
            }
        };
        let room_id =
            MatrixRoomId::new(room.matrix_room_id().as_str().to_owned()).map_err(|_| {
                BridgeIpcDispatchFailure::new(
                    "bridge.lobby_room_invalid",
                    IpcErrorCategory::DependencyUnavailable,
                    false,
                )
            })?;
        // 服务器那边已经进去了；本机还没同步到时照样回答进去了，说话时再按“不在房间里”重试。
        if !entry.joined.wait_for(&room_id, LOCAL_JOIN_WAIT).await {
            tracing::debug!("进了房间，本机的 Matrix 客户端还没同步到");
        }
        Ok(IpcResponse::RoomEntered {
            room: IpcSessionRoom {
                catalog_id: catalog_id.to_string(),
                room_id: room_id.as_str().to_owned(),
            },
        })
    }

    /// 离开一个房间；原本就不在时也回答，`left` 为 `false`。开会话时进的那个房间离不开：等消息、
    /// 说话默认用它，要离开就关会话。
    pub(in crate::ipc) async fn leave_room(
        &self,
        request: IpcLeaveRoomRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let entry = runtime
            .room_entry
            .clone()
            .ok_or_else(agent_runtime_unavailable)?;
        let catalog_id = parse_catalog_id(&request.catalog_id)?;
        if catalog_id == entry.session_room {
            return Err(invalid_request("bridge.leave_session_room"));
        }
        let left = entry
            .lobby
            .leave(&runtime.identity, catalog_id)
            .await
            .map_err(map_lobby_failure)?;
        Ok(IpcResponse::RoomLeft {
            catalog_id: catalog_id.to_string(),
            left,
        })
    }
}

fn parse_catalog_id(value: &str) -> Result<RoomCatalogId, BridgeIpcDispatchFailure> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version() == Some(Version::SortRand))
        .map(RoomCatalogId::from_uuid)
        .ok_or_else(|| invalid_request("bridge.ipc.room_invalid"))
}

const fn retryable(code: &'static str) -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(code, IpcErrorCategory::DependencyUnavailable, true)
}

/// 和开会话时进房间用同一套错误码（`bridge.lobby_*`）。
const fn map_lobby_failure(failure: AgentLobbySessionFailure) -> BridgeIpcDispatchFailure {
    let (code, category, retryable) = match failure.kind() {
        AgentLobbySessionFailureKind::InvalidRequest => (
            "bridge.lobby_request_invalid",
            IpcErrorCategory::Validation,
            false,
        ),
        AgentLobbySessionFailureKind::NotAuthorized => (
            "bridge.lobby_not_authorized",
            IpcErrorCategory::Authentication,
            false,
        ),
        AgentLobbySessionFailureKind::Forbidden => (
            "bridge.lobby_forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        AgentLobbySessionFailureKind::NotFound => (
            "bridge.lobby_not_found",
            IpcErrorCategory::Validation,
            false,
        ),
        AgentLobbySessionFailureKind::Conflict => {
            ("bridge.lobby_conflict", IpcErrorCategory::Conflict, true)
        }
        AgentLobbySessionFailureKind::ControlPlaneUnavailable => (
            "bridge.lobby_control_plane_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        // 进出房间都可以照原样再来一次：再进回答同一个房间，再离开回答 `left: false`。
        AgentLobbySessionFailureKind::EntryOutcomeUnknown => (
            "bridge.lobby_entry_unknown",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        AgentLobbySessionFailureKind::InvalidControlPlaneResponse => (
            "bridge.lobby_response_invalid",
            IpcErrorCategory::DependencyUnavailable,
            false,
        ),
        AgentLobbySessionFailureKind::Internal => {
            ("bridge.lobby_internal", IpcErrorCategory::Internal, false)
        }
    };
    BridgeIpcDispatchFailure::new(code, category, retryable)
}
