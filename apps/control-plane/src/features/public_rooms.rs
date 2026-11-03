//! 发现页（`specs/public-rooms/design.md`），不登录也能读：
//!
//! - `GET /public-rooms?q=&join=&active=&encrypted=&sort=&cursor=`：公开房间的目录。`join` 是
//!   `open` 或 `request`；`active=true` 只要一小时内有人说话的；`encrypted=false` 只要不加密的；
//!   `sort` 是 `active`（默认）、`members` 或 `newest`；`cursor` 照上一页的 `nextCursor` 原样带回。
//! - `GET /public-rooms/{分享码}`：一间房的详情，多几位管理员和 `online`（`count` 个在线，
//!   `atLeast` 是没问全、可能更多；问不到时是 `null`，60 秒算一次）。不公开、关了、没有这个码都回
//!   同一个 404，不写房间名。
//!
//! 带着有效的登录时，每间房多一个 `viewer`：`joined`、`invited`、`pending`（在等批准）或 `banned`；
//! 没登录、登录过期的照样回答，只是没有它。
//!
//! 两个接口和分享页合起来按来源限流（[`AnonymousLimiter`]），超了回 429
//! `public_rooms.rate_limited` 带 `Retry-After`。登录了也照来源算：挡的是挨个猜分享码。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationUseCases},
    ports::{PublicRoomRecord, PublicRoomSort, PublicRoomViewerState},
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicRoomDetailView, PublicRoomDirectoryUseCases, PublicRoomOnline, PublicRoomPage,
        PublicRoomView,
    },
};
use agent_room_domain::{room_listing::RoomJoinMode, time::UtcMillis};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        anonymous_limits::{AnonymousLimiter, Limited},
        authentication::{no_store, session_secret},
    },
    request_regions,
};

#[derive(Clone)]
pub(crate) struct PublicRoomsHttpState {
    pub(crate) directory: Arc<dyn PublicRoomDirectoryUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) limits: Arc<AnonymousLimiter>,
}

pub(crate) fn router(state: PublicRoomsHttpState) -> Router {
    Router::new()
        .route("/public-rooms", get(browse))
        .route("/public-rooms/{share_code}", get(detail))
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
struct BrowseQuery {
    q: Option<String>,
    join: Option<String>,
    active: Option<String>,
    encrypted: Option<String>,
    sort: Option<String>,
    cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageResponse {
    rooms: Vec<RoomResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomResponse {
    catalog_id: String,
    share_code: String,
    name: String,
    description: String,
    owner_name: String,
    member_count: u32,
    agent_count: u32,
    join_mode: &'static str,
    end_to_end_encrypted: bool,
    created_at_unix_ms: i64,
    last_message_at_unix_ms: Option<i64>,
    viewer: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DetailResponse {
    #[serde(flatten)]
    room: RoomResponse,
    admins: Vec<String>,
    online: Option<OnlineResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OnlineResponse {
    count: u32,
    at_least: bool,
}

async fn browse(
    State(state): State<PublicRoomsHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
    headers: HeaderMap,
    query: Result<Query<BrowseQuery>, QueryRejection>,
) -> Response {
    if let Err(limited) = state.limits.admit(&headers) {
        return rate_limited(limited, correlation_id);
    }
    let Some(request) = query.ok().and_then(|Query(query)| browse_request(query)) else {
        return no_store(
            ApiError::invalid_request("public_room.invalid_query", correlation_id).into_response(),
        );
    };
    let request = BrowsePublicRooms {
        viewer: viewer(&state, &jar).await,
        ..request
    };
    match state.directory.browse(request).await {
        Ok(page) => no_store(Json(page_response(page)).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn detail(
    State(state): State<PublicRoomsHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(share_code): Path<String>,
) -> Response {
    if let Err(limited) = state.limits.admit(&headers) {
        return rate_limited(limited, correlation_id);
    }
    let viewer = viewer(&state, &jar).await;
    match state.directory.detail(viewer, share_code).await {
        Ok(detail) => no_store(Json(detail_response(detail)).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

fn rate_limited(limited: Limited, correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "public_rooms.rate_limited",
            ErrorCategory::Transient,
            "请求太频繁，请稍后再试。",
            correlation_id,
        )
        .retry_after_seconds(limited.retry_after_seconds)
        .into_response(),
    )
}

/// 有登录就认，没有、过期了、对不上的都当没登录：这两个接口本来就不要登录。
async fn viewer(state: &PublicRoomsHttpState, jar: &CookieJar) -> Option<AuthenticatedPrincipal> {
    let secret = session_secret(jar).ok()?;
    let principal = state
        .authentication
        .authenticate(&secret, AuthenticationRequirement::ActiveSession)
        .await
        .ok()?;
    request_regions::note_person(principal.principal_id);
    Some(principal)
}

/// 查询参数换成请求；认不出的值回 `None`。登录的人由调用的人补上。
fn browse_request(query: BrowseQuery) -> Option<BrowsePublicRooms> {
    let join_mode = match query.join.as_deref() {
        None | Some("") => None,
        Some("open") => Some(RoomJoinMode::Open),
        Some("request") => Some(RoomJoinMode::Request),
        Some(_) => return None,
    };
    let recently_active = match query.active.as_deref() {
        None | Some("" | "false") => false,
        Some("true") => true,
        Some(_) => return None,
    };
    let unencrypted_only = match query.encrypted.as_deref() {
        None | Some("") => false,
        Some("false") => true,
        Some(_) => return None,
    };
    let sort = match query.sort.as_deref() {
        None | Some("" | "active") => PublicRoomSort::Active,
        Some("members") => PublicRoomSort::Members,
        Some("newest") => PublicRoomSort::Newest,
        Some(_) => return None,
    };
    let offset = match query.cursor.as_deref() {
        None | Some("") => 0,
        Some(cursor) => cursor
            .parse()
            .ok()
            .filter(|_| cursor.bytes().all(|byte| byte.is_ascii_digit()))?,
    };
    Some(BrowsePublicRooms {
        viewer: None,
        text: query.q.unwrap_or_default(),
        join_mode,
        recently_active,
        unencrypted_only,
        sort,
        offset,
    })
}

fn page_response(page: PublicRoomPage) -> PageResponse {
    PageResponse {
        rooms: page.rooms.into_iter().map(room_response).collect(),
        next_cursor: page.next_offset.map(|offset| offset.to_string()),
    }
}

fn detail_response(detail: PublicRoomDetailView) -> DetailResponse {
    DetailResponse {
        room: room_response(detail.room),
        admins: detail.admin_display_names,
        online: detail
            .online
            .map(|PublicRoomOnline { count, at_least }| OnlineResponse { count, at_least }),
    }
}

fn room_response(view: PublicRoomView) -> RoomResponse {
    let PublicRoomRecord {
        catalog_id,
        name,
        description,
        owner_display_name,
        member_count,
        agent_count,
        join_mode,
        end_to_end_encrypted,
        share_code,
        created_at,
        last_message_at,
    } = view.room;
    RoomResponse {
        catalog_id: catalog_id.as_uuid().to_string(),
        share_code: share_code.as_str().to_owned(),
        name,
        description,
        owner_name: owner_display_name,
        member_count,
        agent_count,
        join_mode: join_mode.as_str(),
        end_to_end_encrypted,
        created_at_unix_ms: created_at.value(),
        last_message_at_unix_ms: last_message_at.map(UtcMillis::value),
        viewer: view.viewer.map(viewer_state),
    }
}

const fn viewer_state(state: PublicRoomViewerState) -> &'static str {
    match state {
        PublicRoomViewerState::Joined => "joined",
        PublicRoomViewerState::Invited => "invited",
        PublicRoomViewerState::Pending => "pending",
        PublicRoomViewerState::Banned => "banned",
    }
}

/// 错误码统一以 `public_room.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: PublicDirectoryFailure, correlation_id: CorrelationId) -> Response {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "读公开房间的目录失败"
    );
    let (status, code, category, message) = match failure.kind() {
        PublicDirectoryFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "public_room.invalid_query",
            ErrorCategory::Validation,
            "搜索的字最多 100 个，不能带控制字符；翻页也有上限。",
        ),
        PublicDirectoryFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "public_room.not_found",
            ErrorCategory::Validation,
            "这个房间现在不公开了。",
        ),
        PublicDirectoryFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "public_room.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时读不到公开房间，请稍后再试。",
        ),
        PublicDirectoryFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "public_room.internal",
            ErrorCategory::Transient,
            "读公开房间发生内部错误。",
        ),
    };
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
