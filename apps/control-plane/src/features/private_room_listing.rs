//! 公开房间的设置（`specs/public-rooms/design.md`）：`GET`、`PUT /private-rooms/{c}/listing`。
//! 房间里已加入的人都看得到（公开房间的成员都能分享），只有房主能改。
//!
//! 单独一个接口，不往私人房间的回答里加字段：网页对它 `.strict()` 校验，旧版桌面端多看到一个字段，
//! 整个房间都读不出来。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{
        InspectListing, ListingFailure, ListingFailureKind, ListingView,
        PrivateRoomListingUseCases, UpdateListing,
    },
};
use agent_room_domain::{
    ids::RoomCatalogId,
    room_listing::{RoomJoinMode, RoomListingSettings},
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
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
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        private_rooms::{PermissionsBody, PermissionsResponse},
        public_room_platform_hold::PlatformHoldResponse,
        resource_ids::parse_uuid_v7,
    },
};

const MAX_LISTING_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomListingHttpState {
    listings: Arc<dyn PrivateRoomListingUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl PrivateRoomListingHttpState {
    pub(crate) fn new(
        listings: Arc<dyn PrivateRoomListingUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            listings,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PrivateRoomListingHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/listing",
            get(inspect).put(update),
        )
        .layer(DefaultBodyLimit::max(MAX_LISTING_BODY_BYTES))
        .with_state(state)
}

/// 公开设置的请求正文。新建房间时选公开也用它（`POST /private-rooms` 的 `listing`）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListingBody {
    listed: bool,
    join_mode: JoinModeWire,
    member_permissions: PermissionsBody,
    allow_network_agents: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JoinModeWire {
    Open,
    Request,
    InviteOnly,
}

impl From<JoinModeWire> for RoomJoinMode {
    fn from(mode: JoinModeWire) -> Self {
        match mode {
            JoinModeWire::Open => Self::Open,
            JoinModeWire::Request => Self::Request,
            JoinModeWire::InviteOnly => Self::InviteOnly,
        }
    }
}

impl From<RoomJoinMode> for JoinModeWire {
    fn from(mode: RoomJoinMode) -> Self {
        match mode {
            RoomJoinMode::Open => Self::Open,
            RoomJoinMode::Request => Self::Request,
            RoomJoinMode::InviteOnly => Self::InviteOnly,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListingResponse {
    listed: bool,
    join_mode: JoinModeWire,
    member_permissions: PermissionsResponse,
    allow_network_agents: bool,
    /// 从没公开过的房间没有分享码。
    share_code: Option<String>,
    first_listed_at_unix_ms: Option<i64>,
    end_to_end_encrypted: bool,
    /// 平台把房间从目录里拿掉了：原因和时间。没拿掉是 `null`。
    platform_hold: Option<PlatformHoldResponse>,
}

impl From<ListingView> for ListingResponse {
    fn from(view: ListingView) -> Self {
        Self {
            listed: view.settings.listed(),
            join_mode: view.settings.join_mode().into(),
            member_permissions: PermissionsResponse::from(view.settings.member_permissions()),
            allow_network_agents: view.settings.allow_network_agents(),
            share_code: view.share_code.map(|code| code.as_str().to_owned()),
            first_listed_at_unix_ms: view.first_listed_at.map(UtcMillis::value),
            end_to_end_encrypted: view.end_to_end_encrypted,
            platform_hold: view
                .platform_hold
                .map(|hold| PlatformHoldResponse::new(hold.reason, hold.held_at)),
        }
    }
}

async fn inspect(
    State(state): State<PrivateRoomListingHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("room_listing.invalid_resource", correlation_id);
    };
    let actor = match session(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    listing_response(
        state
            .listings
            .inspect(InspectListing { actor, catalog_id })
            .await,
        correlation_id,
    )
}

async fn update(
    State(state): State<PrivateRoomListingHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Some(settings) = parse_settings(&body) else {
        return invalid("room_listing.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_listing.invalid_origin",
                ErrorCategory::Authorization,
                "公开设置的请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let actor = match session(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("room_listing.invalid_resource", correlation_id);
    };
    listing_response(
        state
            .listings
            .update(UpdateListing {
                actor,
                catalog_id,
                settings,
            })
            .await,
        correlation_id,
    )
}

impl ListingBody {
    pub(crate) fn into_settings(self) -> Option<RoomListingSettings> {
        RoomListingSettings::new(
            self.listed,
            self.join_mode.into(),
            self.member_permissions.into_domain()?,
            self.allow_network_agents,
        )
        .ok()
    }
}

fn parse_settings(body: &[u8]) -> Option<RoomListingSettings> {
    serde_json::from_slice::<ListingBody>(body)
        .ok()?
        .into_settings()
}

async fn session(
    state: &PrivateRoomListingHttpState,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
}

fn listing_response(
    result: Result<ListingView, ListingFailure>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(view) => no_store((StatusCode::OK, Json(ListingResponse::from(view))).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_listing.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: ListingFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        ListingFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "room_listing.invalid",
            ErrorCategory::Validation,
            "公开设置不对：新成员要能查看，不能带着管理。",
        ),
        ListingFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_listing.forbidden",
            ErrorCategory::Authorization,
            "只有房主能改公开设置，或者登录已失效。",
        ),
        ListingFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_listing.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在房间里。",
        ),
        ListingFailureKind::LimitReached => (
            StatusCode::CONFLICT,
            "room_listing.limit_reached",
            ErrorCategory::Conflict,
            "每个账户只能有一间公开房间，先把另一间改回私密。",
        ),
        ListingFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_listing.conflict",
            ErrorCategory::Conflict,
            "房间的设置刚刚变了，请再试一次。",
        ),
        ListingFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_listing.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "公开设置暂时没改成，请稍后再点一次。",
        ),
        ListingFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_listing.internal",
            ErrorCategory::Transient,
            "公开设置发生内部错误。",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "公开设置请求失败"
    );
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
