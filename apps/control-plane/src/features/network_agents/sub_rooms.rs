//! 网络 Agent 看一间房下面的子房间（`specs/redesign/sub-rooms.md` 第 2h-2 步）：
//! `GET /v1/network-agents/me/private-rooms/{room}/sub-rooms`，远程 MCP 的 `agent_room_list_rooms`
//! 带上 `parent` 时回同一个形状。看得到哪几间由用例定，这里只管回答的样子。

use agent_room_application::network_agents::{NetworkAgentSubRoomList, NetworkAgentSubRoomSummary};
use agent_room_domain::time::UtcMillis;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json,
    extract::{Extension, Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{NetworkAgentHttpState, RoomResponse, SCHEMA_VERSION, rooms::agent_failure};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{authentication::no_store, devices::bearer_secret},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SubRoomsQuery {
    /// true 时只列归档了的。
    #[serde(default)]
    archived: bool,
}

/// 一间房下面它看得到的子房间。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SubRoomListResponse {
    schema_version: u8,
    parent: RoomResponse,
    /// 父房间里的分类，按排好的先后。
    categories: Vec<String>,
    sub_rooms: Vec<SubRoomResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomResponse {
    room: RoomResponse,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    /// “未分类”的没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    /// `open`、`request` 或 `invite_only`。
    access: &'static str,
    joined: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    created_by_you: bool,
    /// 下面三项只在没有门的、或者它在里面的子房间上有。
    #[serde(skip_serializing_if = "Option::is_none")]
    people: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agents: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_message_at_unix_ms: Option<i64>,
}

impl From<NetworkAgentSubRoomSummary> for SubRoomResponse {
    fn from(summary: NetworkAgentSubRoomSummary) -> Self {
        let activity = summary.activity;
        Self {
            room: RoomResponse::from(summary.room),
            description: summary.description,
            category: summary.category,
            access: summary.access.as_str(),
            joined: summary.joined,
            created_by_you: summary.created_by_you,
            people: activity.map(|activity| activity.people),
            agents: activity.map(|activity| activity.agents),
            last_message_at_unix_ms: activity
                .and_then(|activity| activity.last_message_at)
                .map(UtcMillis::value),
        }
    }
}

pub(super) fn sub_room_list(list: NetworkAgentSubRoomList) -> SubRoomListResponse {
    SubRoomListResponse {
        schema_version: SCHEMA_VERSION,
        parent: RoomResponse::from(list.parent),
        categories: list.categories,
        sub_rooms: list.rooms.into_iter().map(SubRoomResponse::from).collect(),
    }
}

/// 一间房下面它看得到的子房间：`room` 是它建的或凭口令、敲门进了的房间（也收它下面的子房间），
/// 房间号或 Matrix 房间 ID。
pub(super) async fn sub_rooms(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    query: Result<Query<SubRoomsQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_request",
                ErrorCategory::Validation,
                "The only query parameter is archived (true or false): true lists the sub-rooms that were archived.",
                correlation_id,
            )
            .into_response(),
        );
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state
        .owned_rooms
        .list_sub_rooms(token, &room, query.archived)
        .await
    {
        Ok(list) => no_store(Json(sub_room_list(list)).into_response()),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}
