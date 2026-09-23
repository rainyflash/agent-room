//! 凭私人房间口令进房间。查看直接问控制面；兑换先按会话键建好（或找回）宿主人物，再让它凭口令
//! 加入。之后开会话仍走原来的路径，进的就是兑换出的房间。

use std::sync::Arc;

use agent_room_bridge_core::{
    join_codes::{ControlPlaneJoinCodeGateway, JoinCodeFailure, JoinCodeFailureKind, JoinCodeRoom},
    onboarding::HostAgentRegistrationGateway,
};
use agent_room_bridge_ipc::{
    IpcErrorCategory, IpcRedeemJoinCodeRequest, IpcResolveJoinCodeRequest, IpcResponse,
    IpcRoomKind, IpcRoomSummary,
};
use agent_room_domain::ids::AgentCreationRequestId;

use super::registration_failure;
use crate::ipc::BridgeIpcDispatchFailure;

pub(crate) struct JoinCodeAccess {
    pub(crate) gateway: Arc<dyn ControlPlaneJoinCodeGateway>,
    pub(crate) host_agents: Arc<dyn HostAgentRegistrationGateway>,
}

impl JoinCodeAccess {
    pub(crate) async fn resolve(
        &self,
        request: IpcResolveJoinCodeRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        self.gateway
            .resolve(&request.code)
            .await
            .map(room_response)
            .map_err(join_code_failure)
    }

    pub(crate) async fn redeem(
        &self,
        request: IpcRedeemJoinCodeRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let key = uuid::Uuid::parse_str(&request.session_key).map_err(|_| {
            BridgeIpcDispatchFailure::new(
                "bridge.host_session.key_invalid",
                IpcErrorCategory::Validation,
                false,
            )
        })?;
        // 与开会话同一个会话键、同一个名字：控制面按会话键幂等，这里建出的就是随后开会话的人物。
        let agent = self
            .host_agents
            .create_host_agent(
                AgentCreationRequestId::from_uuid(key),
                &request.display_name,
            )
            .await
            .map_err(|failure| registration_failure(failure.kind()))?;
        self.gateway
            .redeem(agent.agent_id, &request.code)
            .await
            .map(room_response)
            .map_err(join_code_failure)
    }
}

fn room_response(room: JoinCodeRoom) -> IpcResponse {
    IpcResponse::JoinCodeRoom {
        room: IpcRoomSummary {
            kind: IpcRoomKind::PrivateRoom,
            catalog_id: room.catalog_id.to_string(),
            matrix_room_id: Some(room.matrix_room_id.as_str().to_owned()),
            name: room.name,
            slug: None,
            membership: None,
        },
    }
}

fn join_code_failure(failure: JoinCodeFailure) -> BridgeIpcDispatchFailure {
    let (code, category, retryable) = match failure.kind() {
        JoinCodeFailureKind::InvalidCode => (
            "bridge.join_code.invalid",
            IpcErrorCategory::Validation,
            false,
        ),
        JoinCodeFailureKind::NotFound => (
            "bridge.join_code.not_found",
            IpcErrorCategory::Validation,
            false,
        ),
        JoinCodeFailureKind::Forbidden => (
            "bridge.join_code.forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        JoinCodeFailureKind::RoomUnavailable => (
            "bridge.join_code.room_unavailable",
            IpcErrorCategory::Conflict,
            false,
        ),
        // 要等上一阵子才能再试；不标成可重试，免得调用方立刻重发。
        JoinCodeFailureKind::RateLimited => (
            "bridge.join_code.rate_limited",
            IpcErrorCategory::Authorization,
            false,
        ),
        JoinCodeFailureKind::Unsupported => (
            "bridge.join_code.unsupported",
            IpcErrorCategory::IncompatibleVersion,
            false,
        ),
        JoinCodeFailureKind::NotAuthorized => (
            "bridge.host_session.device_authorization_required",
            IpcErrorCategory::Authentication,
            false,
        ),
        JoinCodeFailureKind::ControlPlaneUnavailable => (
            "bridge.join_code.unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        JoinCodeFailureKind::InvalidControlPlaneResponse | JoinCodeFailureKind::Internal => (
            "bridge.join_code.failed",
            IpcErrorCategory::DependencyUnavailable,
            false,
        ),
    };
    BridgeIpcDispatchFailure::new(code, category, retryable)
}
