//! Agent 自己建房间、管房间（`specs/agent-rooms/design.md`）。建房先按会话键建好（或找回）宿主人物，
//! 拿它当建房的 Agent；之后开会话照常走原来的路径，进的就是建好的房间。管房间管的是会话所在的房间：
//! 口令、敲门、邀请人、移出 Agent 都交给控制面，只管 Agent 建的房间由控制面把关。

use std::sync::Arc;

use agent_room_bridge_core::{
    onboarding::HostAgentRegistrationGateway,
    room_management::{
        ControlPlaneRoomManagementGateway, CreatedRoom, RoomCodeState, RoomKnock,
        RoomManagementFailure, RoomManagementFailureKind,
    },
};
use agent_room_bridge_ipc::{
    IpcCreateRoomRequest, IpcErrorCategory, IpcMethod, IpcResponse, IpcRoomAccess,
    IpcRoomAccessAction, IpcRoomCode, IpcRoomKind, IpcRoomKnock, IpcRoomSummary,
};
use agent_room_domain::ids::{AgentCreationRequestId, AgentId, PrincipalId, RoomCatalogId};
use uuid::Uuid;

use super::registration_failure;
use crate::ipc::BridgeIpcDispatchFailure;

pub(crate) struct RoomManagementAccess {
    pub(crate) gateway: Arc<dyn ControlPlaneRoomManagementGateway>,
    pub(crate) host_agents: Arc<dyn HostAgentRegistrationGateway>,
    /// 接入说明的地址：Agent 把口令转给别的 Agent 时一起带上。
    pub(crate) guide_url: String,
}

/// 管会话所在房间的方法：先要知道会话在哪个房间。
pub(crate) const fn manages_room(method: &IpcMethod) -> bool {
    matches!(
        method,
        IpcMethod::RoomAccess(_)
            | IpcMethod::AnswerKnock(_)
            | IpcMethod::InvitePerson(_)
            | IpcMethod::RemoveAgent(_)
    )
}

impl RoomManagementAccess {
    pub(crate) async fn create(
        &self,
        request: IpcCreateRoomRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let key = parse(&request.session_key, "bridge.host_session.key_invalid")?;
        let catalog_id =
            RoomCatalogId::from_uuid(parse(&request.catalog_id, "bridge.ipc.room_invalid")?);
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
            .create(catalog_id, &request.name, agent.agent_id)
            .await
            .map(|room| IpcResponse::CreatedRoom {
                room: created_summary(room),
            })
            .map_err(room_failure)
    }

    /// 管会话所在的房间，只收 [`manages_room`] 认的方法。
    pub(crate) async fn manage(
        &self,
        catalog_id: RoomCatalogId,
        method: IpcMethod,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let done = match method {
            IpcMethod::RoomAccess(request) => return self.access(catalog_id, request.action).await,
            IpcMethod::AnswerKnock(request) => {
                let agent_id =
                    AgentId::from_uuid(parse(&request.agent_id, "bridge.ipc.agent_id_invalid")?);
                self.gateway
                    .answer_knock(catalog_id, agent_id, request.admit)
                    .await
            }
            IpcMethod::InvitePerson(request) => {
                let account_id = PrincipalId::from_uuid(parse(
                    &request.account_id,
                    "bridge.ipc.account_id_invalid",
                )?);
                self.gateway.invite(catalog_id, account_id).await
            }
            IpcMethod::RemoveAgent(request) => {
                let agent_id =
                    AgentId::from_uuid(parse(&request.agent_id, "bridge.ipc.agent_id_invalid")?);
                self.gateway.remove_agent(catalog_id, agent_id).await
            }
            _ => {
                return Err(BridgeIpcDispatchFailure::new(
                    "bridge.ipc.session_method_invalid",
                    IpcErrorCategory::Validation,
                    false,
                ));
            }
        };
        done.map_err(room_failure)?;
        Ok(IpcResponse::RoomUpdated {
            catalog_id: catalog_id.to_string(),
        })
    }

    async fn access(
        &self,
        catalog_id: RoomCatalogId,
        action: IpcRoomAccessAction,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let code = match action {
            IpcRoomAccessAction::Show => {
                match self.gateway.code(catalog_id).await.map_err(room_failure)? {
                    RoomCodeState::Visible(code) => IpcRoomCode::Visible { code: code.code },
                    // 看不到的不替人换：换了以后，已经拿到旧口令、还没进来的 Agent 就进不来了。
                    RoomCodeState::Hidden => IpcRoomCode::Hidden,
                    RoomCodeState::Missing => self.generate(catalog_id).await?,
                }
            }
            IpcRoomAccessAction::Replace => self.generate(catalog_id).await?,
            IpcRoomAccessAction::Disable => {
                self.gateway
                    .disable_code(catalog_id)
                    .await
                    .map_err(room_failure)?;
                IpcRoomCode::Missing
            }
        };
        let knocks = self
            .gateway
            .knocks(catalog_id)
            .await
            .map_err(room_failure)?;
        Ok(IpcResponse::RoomAccess {
            access: IpcRoomAccess {
                catalog_id: catalog_id.to_string(),
                code,
                knocks: knocks.into_iter().map(knock_summary).collect(),
                guide_url: self.guide_url.clone(),
            },
        })
    }

    async fn generate(
        &self,
        catalog_id: RoomCatalogId,
    ) -> Result<IpcRoomCode, BridgeIpcDispatchFailure> {
        self.gateway
            .generate_code(catalog_id)
            .await
            .map(|code| IpcRoomCode::Visible { code: code.code })
            .map_err(room_failure)
    }
}

/// 请求进 Bridge 之前已经按 IPC 的规矩核对过，这里只是换成类型。
fn parse(value: &str, code: &'static str) -> Result<Uuid, BridgeIpcDispatchFailure> {
    Uuid::parse_str(value)
        .map_err(|_| BridgeIpcDispatchFailure::new(code, IpcErrorCategory::Validation, false))
}

fn created_summary(room: CreatedRoom) -> IpcRoomSummary {
    IpcRoomSummary {
        kind: IpcRoomKind::PrivateRoom,
        catalog_id: room.catalog_id.to_string(),
        matrix_room_id: Some(room.matrix_room_id.as_str().to_owned()),
        name: room.name,
        slug: None,
        membership: None,
    }
}

fn knock_summary(knock: RoomKnock) -> IpcRoomKnock {
    IpcRoomKnock {
        agent_id: knock.agent_id.to_string(),
        display_name: knock.display_name,
        knocked_at_unix_ms: knock.knocked_at.value(),
        expires_at_unix_ms: knock.expires_at.value(),
    }
}

fn room_failure(failure: RoomManagementFailure) -> BridgeIpcDispatchFailure {
    let (code, category, retryable) = match failure.kind() {
        RoomManagementFailureKind::InvalidRequest => (
            "bridge.rooms.invalid_request",
            IpcErrorCategory::Validation,
            false,
        ),
        RoomManagementFailureKind::Forbidden => (
            "bridge.rooms.forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        RoomManagementFailureKind::NotFound => (
            "bridge.rooms.not_found",
            IpcErrorCategory::Validation,
            false,
        ),
        RoomManagementFailureKind::Conflict => {
            ("bridge.rooms.conflict", IpcErrorCategory::Conflict, false)
        }
        // 要等上一阵子才能再试；不标成可重试，免得调用方立刻重发。
        RoomManagementFailureKind::RateLimited => (
            "bridge.rooms.rate_limited",
            IpcErrorCategory::Authorization,
            false,
        ),
        RoomManagementFailureKind::Unsupported => (
            "bridge.rooms.unsupported",
            IpcErrorCategory::IncompatibleVersion,
            false,
        ),
        RoomManagementFailureKind::NotAuthorized => (
            "bridge.host_session.device_authorization_required",
            IpcErrorCategory::Authentication,
            false,
        ),
        RoomManagementFailureKind::ControlPlaneUnavailable => (
            "bridge.rooms.control_plane_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        RoomManagementFailureKind::InvalidControlPlaneResponse
        | RoomManagementFailureKind::Internal => (
            "bridge.rooms.failed",
            IpcErrorCategory::DependencyUnavailable,
            false,
        ),
    };
    BridgeIpcDispatchFailure::new(code, category, retryable)
}
