//! Agent 在哪些房间、离开一个房间（`specs/agent-identity/design.md` 第 2 步）。
//!
//! 设备签名的离开和进房间那条用同一个路径（`DELETE` 对 `POST`），Bridge 用；主人用的两条是新的
//! 接口：不往现有的回答里加字段，`agentInstanceSchema` 和 Agent 列表的回答都是 `.strict()`。

use std::sync::Arc;

use agent_room_application::{
    agent_room_placements::{
        AgentRoom, AgentRoomActor, AgentRoomPlacementUseCases, LeaveAgentRoom, LeftAgentRoom,
        ListAgentRooms,
    },
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    devices::DeviceAuthorizationUseCases,
    ports::{AgentRoomKind, SecretFactory},
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    rooms::RoomCatalogStatus,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

#[derive(Clone)]
pub(crate) struct AgentRoomHttpState {
    placements: Arc<dyn AgentRoomPlacementUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct AgentRoomHttpDependencies {
    pub(crate) placements: Arc<dyn AgentRoomPlacementUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl AgentRoomHttpState {
    pub(crate) fn new(
        dependencies: AgentRoomHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            placements: dependencies.placements,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

/// 设备签名的离开挂在进房间那条路径上，和大厅路由合并以后同一个路径两个方法。
pub(crate) fn router(state: AgentRoomHttpState) -> Router {
    Router::new()
        .route(
            "/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry",
            delete(leave_by_device),
        )
        .route("/agents/{agent_id}/rooms", get(list_rooms))
        .route(
            "/agents/{agent_id}/rooms/{catalog_id}",
            delete(leave_by_owner),
        )
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRoomsResponse {
    rooms: Vec<AgentRoomResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRoomResponse {
    catalog_id: String,
    kind: &'static str,
    name: String,
    archived: bool,
}

impl From<AgentRoom> for AgentRoomResponse {
    fn from(room: AgentRoom) -> Self {
        Self {
            catalog_id: room.catalog_id.to_string(),
            kind: match room.kind {
                AgentRoomKind::PublicLobby => "public_lobby",
                AgentRoomKind::PrivateRoom => "private_room",
            },
            name: room.name,
            archived: room.status == RoomCatalogStatus::Archived,
        }
    }
}

/// 原本就不在这个房间时 `left` 为 `false`，也是 200：重复离开不报错。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LeaveResponse {
    left: bool,
}

async fn leave_by_device(
    State(state): State<AgentRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((agent_id, instance_id, catalog_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target =
        format!("/agents/{agent_id}/instances/{instance_id}/lobbies/{catalog_id}/entry");
    let Ok(agent_id) = parse_uuid_v7(&agent_id).map(AgentId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(catalog_id) = parse_uuid_v7(&catalog_id).map(RoomCatalogId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    if !body.is_empty() {
        return no_store(
            ApiError::invalid_request("lobby.invalid_entry_body", correlation_id).into_response(),
        );
    }
    let device = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "DELETE",
        &request_target,
        "",
        correlation_id,
    )
    .await
    {
        Ok(device) => device,
        Err(response) => return response,
    };
    leave(
        &state,
        correlation_id,
        LeaveAgentRoom {
            actor: AgentRoomActor::Device {
                device,
                agent_instance_id: instance_id,
            },
            agent_id,
            catalog_id,
        },
    )
    .await
}

async fn list_rooms(
    State(state): State<AgentRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(agent_id): Path<String>,
    jar: CookieJar,
) -> Response {
    let Ok(agent_id) = parse_uuid_v7(&agent_id).map(AgentId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .placements
        .rooms(ListAgentRooms { actor, agent_id })
        .await
    {
        Ok(rooms) => no_store(
            Json(AgentRoomsResponse {
                rooms: rooms.into_iter().map(AgentRoomResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(ApiError::agent_lobby(&failure, correlation_id).into_response()),
    }
}

async fn leave_by_owner(
    State(state): State<AgentRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((agent_id, catalog_id)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "agent_room.invalid_origin",
                ErrorCategory::Authorization,
                "请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let Ok(agent_id) = parse_uuid_v7(&agent_id).map(AgentId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(catalog_id) = parse_uuid_v7(&catalog_id).map(RoomCatalogId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    leave(
        &state,
        correlation_id,
        LeaveAgentRoom {
            actor: AgentRoomActor::Owner(actor),
            agent_id,
            catalog_id,
        },
    )
    .await
}

async fn leave(
    state: &AgentRoomHttpState,
    correlation_id: CorrelationId,
    request: LeaveAgentRoom,
) -> Response {
    match state.placements.leave(request).await {
        Ok(LeftAgentRoom { left }) => no_store(Json(LeaveResponse { left }).into_response()),
        Err(failure) => no_store(ApiError::agent_lobby(&failure, correlation_id).into_response()),
    }
}

fn invalid_resource_id(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("agent_room.invalid_resource_id", correlation_id)
}

#[cfg(test)]
mod tests;
