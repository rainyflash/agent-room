//! 网络 Agent 找公开房间（`specs/public-rooms/design.md` 5c）：`GET /v1/network-agents/public-rooms`
//! 和远程 MCP 的 `agent_room_list_rooms`（`publicRooms`）。
//!
//! 只列没有账号的网络 Agent 拿房间号进得去的：房主允许网络 Agent、不是只能邀请的。不要令牌，和发现页
//! 读同一个目录、合起来按来源限流（[`AnonymousLimiter`]）。总开关关着时回 `network_agent.disabled`，
//! 和列大厅一样。

use std::sync::Arc;

use agent_room_application::{
    network_agents::{NetworkAgentFailure, NetworkAgentFailureKind},
    ports::{PublicRoomRecord, PublicRoomSort},
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicRoomDirectoryUseCases, PublicRoomPage,
    },
};
use agent_room_domain::time::UtcMillis;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json,
    extract::{Extension, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{NetworkAgentHttpState, SCHEMA_VERSION};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        anonymous_limits::AnonymousLimiter, authentication::no_store, public_rooms::page_offset,
        share_page::share_url,
    },
};

/// 发现页的目录和限流，网络 Agent 这边共用同一份。
#[derive(Clone)]
pub(crate) struct NetworkAgentPublicRooms {
    pub(crate) directory: Arc<dyn PublicRoomDirectoryUseCases>,
    pub(crate) limits: Arc<AnonymousLimiter>,
    /// 分享链接所在的网站，`https://<Matrix 服务器名>`。
    pub(crate) site_origin: Arc<str>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct PublicRoomsQuery {
    q: Option<String>,
    cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicRoomsResponse {
    schema_version: u8,
    rooms: Vec<PublicRoomResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

/// 给 Agent 看的一间公开房间。`room` 是房间号，原样交给加入的 `room`。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicRoomResponse {
    room: String,
    name: String,
    description: String,
    owner_name: String,
    join_mode: &'static str,
    end_to_end_encrypted: bool,
    member_count: u32,
    agent_count: u32,
    last_message_at_unix_ms: Option<i64>,
    share_url: String,
}

pub(super) async fn public_rooms(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    query: Result<Query<PublicRoomsQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(invalid_query(correlation_id).into_response());
    };
    match browse(&state, &headers, query.q, query.cursor, correlation_id).await {
        Ok(page) => no_store(Json(page).into_response()),
        Err(error) => no_store(error.into_response()),
    }
}

/// 翻一页公开房间：最近有人说话的在前，`cursor` 照上一页的 `nextCursor` 原样带回。
pub(super) async fn browse(
    state: &NetworkAgentHttpState,
    headers: &HeaderMap,
    search: Option<String>,
    cursor: Option<String>,
    correlation_id: CorrelationId,
) -> Result<PublicRoomsResponse, ApiError> {
    let Some(public_rooms) = &state.public_rooms else {
        return Err(ApiError::network_agent(
            &NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled),
            correlation_id,
        ));
    };
    if let Err(limited) = public_rooms.limits.admit(headers) {
        return Err(ApiError::network_agent(
            &NetworkAgentFailure::new(NetworkAgentFailureKind::RateLimited),
            correlation_id,
        )
        .retry_after_seconds(limited.retry_after_seconds));
    }
    let offset = page_offset(cursor.as_deref()).ok_or_else(|| invalid_query(correlation_id))?;
    let page = public_rooms
        .directory
        .browse(BrowsePublicRooms {
            viewer: None,
            text: search.unwrap_or_default(),
            join_mode: None,
            recently_active: false,
            unencrypted_only: false,
            network_agents_only: true,
            sort: PublicRoomSort::Active,
            offset,
        })
        .await
        .map_err(|failure| directory_failure(failure, correlation_id))?;
    Ok(page_response(page, &public_rooms.site_origin))
}

fn page_response(page: PublicRoomPage, site_origin: &str) -> PublicRoomsResponse {
    PublicRoomsResponse {
        schema_version: SCHEMA_VERSION,
        rooms: page
            .rooms
            .into_iter()
            .map(|view| room_response(view.room, site_origin))
            .collect(),
        next_cursor: page.next_offset.map(|offset| offset.to_string()),
    }
}

fn room_response(room: PublicRoomRecord, site_origin: &str) -> PublicRoomResponse {
    PublicRoomResponse {
        room: room.catalog_id.as_uuid().to_string(),
        share_url: share_url(site_origin, room.share_code.as_str()),
        name: room.name,
        description: room.description,
        owner_name: room.owner_display_name,
        join_mode: room.join_mode.as_str(),
        end_to_end_encrypted: room.end_to_end_encrypted,
        member_count: room.member_count,
        agent_count: room.agent_count,
        last_message_at_unix_ms: room.last_message_at.map(UtcMillis::value),
    }
}

fn invalid_query(correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "network_agent.invalid_request",
        ErrorCategory::Validation,
        "search is at most 100 characters with no control characters, and cursor must be the nextCursor of the previous page, unchanged.",
        correlation_id,
    )
}

fn directory_failure(failure: PublicDirectoryFailure, correlation_id: CorrelationId) -> ApiError {
    let kind = match failure.kind() {
        PublicDirectoryFailureKind::InvalidRequest => return invalid_query(correlation_id),
        PublicDirectoryFailureKind::DependencyUnavailable => {
            NetworkAgentFailureKind::DependencyUnavailable
        }
        PublicDirectoryFailureKind::NotFound | PublicDirectoryFailureKind::Internal => {
            NetworkAgentFailureKind::Internal
        }
    };
    ApiError::network_agent(&NetworkAgentFailure::new(kind), correlation_id)
}
