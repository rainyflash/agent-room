//! Agent 自己建房间、管房间（`specs/agent-rooms/design.md`）。建房先按会话键建好（或找回）宿主人物，
//! 拿它当建房的 Agent；之后开会话照常走原来的路径，进的就是建好的房间。管房间管的是会话所在的房间：
//! 口令、敲门、邀请人、移出 Agent 都交给控制面，只管 Agent 建的房间由控制面把关。

use std::sync::Arc;

use agent_room_bridge_core::{
    onboarding::HostAgentRegistrationGateway,
    room_management::{
        ControlPlaneRoomManagementGateway, CreatedRoom, RoomCodeState, RoomKnock,
        RoomManagementFailure, RoomManagementFailureKind, SubRoomPlacement,
    },
};
use agent_room_bridge_ipc::{
    IpcCreateRoomRequest, IpcErrorCategory, IpcMethod, IpcParentRoom, IpcPublicRoomJoinMode,
    IpcResponse, IpcRoomAccess, IpcRoomAccessAction, IpcRoomCode, IpcRoomKind, IpcRoomKnock,
    IpcRoomSummary,
};
use agent_room_domain::{
    ids::{AgentCreationRequestId, AgentId, PrincipalId, RoomCatalogId},
    room_listing::RoomJoinMode,
};
use uuid::Uuid;

use super::{ipc_join_mode, registration_failure};
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
        let sub_room = request
            .parent_catalog_id
            .as_deref()
            .map(|parent| {
                Ok::<_, BridgeIpcDispatchFailure>(SubRoomPlacement {
                    parent: RoomCatalogId::from_uuid(parse(parent, "bridge.ipc.room_invalid")?),
                    access: request.access.map_or(RoomJoinMode::Open, join_mode),
                    category: request.category.clone(),
                })
            })
            .transpose()?;
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
            .create(catalog_id, &request.name, agent.agent_id, sub_room.as_ref())
            .await
            .map(|room| IpcResponse::CreatedRoom {
                category_missing: room.category_missing,
                room: created_summary(room, sub_room.as_ref()),
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

const fn join_mode(mode: IpcPublicRoomJoinMode) -> RoomJoinMode {
    match mode {
        IpcPublicRoomJoinMode::Open => RoomJoinMode::Open,
        IpcPublicRoomJoinMode::Request => RoomJoinMode::Request,
        IpcPublicRoomJoinMode::InviteOnly => RoomJoinMode::InviteOnly,
    }
}

/// 子房间另带它在哪间房下面、放进的分类和门（门照建的时候要的）。
fn created_summary(room: CreatedRoom, sub_room: Option<&SubRoomPlacement>) -> IpcRoomSummary {
    IpcRoomSummary {
        kind: IpcRoomKind::PrivateRoom,
        catalog_id: room.catalog_id.to_string(),
        matrix_room_id: Some(room.matrix_room_id.as_str().to_owned()),
        name: room.name,
        slug: None,
        membership: None,
        parent: room.parent.map(|parent| IpcParentRoom {
            catalog_id: parent.catalog_id.to_string(),
            name: parent.name,
        }),
        category: room.category,
        access: sub_room.map(|placement| ipc_join_mode(placement.access)),
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

/// 建子房间被拒的几种，照原因分开说。
const fn sub_room_creation_failure(
    kind: RoomManagementFailureKind,
) -> (&'static str, IpcErrorCategory) {
    match kind {
        RoomManagementFailureKind::SubRoomNested => {
            ("bridge.rooms.sub_room_nested", IpcErrorCategory::Validation)
        }
        RoomManagementFailureKind::SubRoomAdminsOnly => (
            "bridge.rooms.sub_room_admins_only",
            IpcErrorCategory::Authorization,
        ),
        RoomManagementFailureKind::SubRoomAgentsMayNotCreate => (
            "bridge.rooms.sub_room_agents_may_not_create",
            IpcErrorCategory::Authorization,
        ),
        RoomManagementFailureKind::SubRoomNameTaken => (
            "bridge.rooms.sub_room_name_taken",
            IpcErrorCategory::Conflict,
        ),
        _ => (
            "bridge.rooms.sub_room_limit_reached",
            IpcErrorCategory::Conflict,
        ),
    }
}

pub(super) fn room_failure(failure: RoomManagementFailure) -> BridgeIpcDispatchFailure {
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
        // 照管理授权动手时的几种：说清楚为什么不行，Agent 好照着办。
        RoomManagementFailureKind::NotGranted => (
            "bridge.moderation.not_granted",
            IpcErrorCategory::Authorization,
            false,
        ),
        RoomManagementFailureKind::NotAllowed => (
            "bridge.moderation.not_allowed",
            IpcErrorCategory::Authorization,
            false,
        ),
        RoomManagementFailureKind::Protected => (
            "bridge.moderation.protected",
            IpcErrorCategory::Authorization,
            false,
        ),
        RoomManagementFailureKind::TargetNotFound => (
            "bridge.moderation.target_not_found",
            IpcErrorCategory::Validation,
            false,
        ),
        RoomManagementFailureKind::NotInRoom => (
            "bridge.moderation.not_in_room",
            IpcErrorCategory::Conflict,
            false,
        ),
        RoomManagementFailureKind::NotBanned => (
            "bridge.moderation.not_banned",
            IpcErrorCategory::Conflict,
            false,
        ),
        RoomManagementFailureKind::BannedByPerson => (
            "bridge.moderation.banned_by_person",
            IpcErrorCategory::Authorization,
            false,
        ),
        kind @ (RoomManagementFailureKind::SubRoomNested
        | RoomManagementFailureKind::SubRoomAdminsOnly
        | RoomManagementFailureKind::SubRoomAgentsMayNotCreate
        | RoomManagementFailureKind::SubRoomNameTaken
        | RoomManagementFailureKind::SubRoomLimitReached) => {
            let (code, category) = sub_room_creation_failure(kind);
            (code, category, false)
        }
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
