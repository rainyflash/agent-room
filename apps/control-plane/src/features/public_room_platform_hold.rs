//! 平台把公开房间从目录里拿掉（`specs/public-rooms/design.md` 的 3b-4b）：
//! `PUT /private-rooms/{c}/listing/platform-hold`（正文 `{"reason": "..."}`，可以空着）拿掉，
//! `DELETE` 同一个地址放回去。只有平台治理的人，要最近登录过，和别的治理一样。
//!
//! 房间里的人在公开设置（`GET /private-rooms/{c}/listing` 的 `platformHold`）里看得到原因和时间。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{
        HoldPublicRoom, PlatformHoldFailure, PlatformHoldFailureKind,
        PublicRoomPlatformHoldUseCases, ReleasePublicRoom,
    },
};
use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::put,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

/// 原因最多 500 个字，一个字最多 4 个字节，再留出 JSON 转义的余量。
const MAX_HOLD_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct PublicRoomPlatformHoldHttpState {
    holds: Arc<dyn PublicRoomPlatformHoldUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl PublicRoomPlatformHoldHttpState {
    pub(crate) fn new(
        holds: Arc<dyn PublicRoomPlatformHoldUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            holds,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PublicRoomPlatformHoldHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/listing/platform-hold",
            put(hold).delete(release),
        )
        .layer(DefaultBodyLimit::max(MAX_HOLD_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldBody {
    reason: String,
}

/// 公开设置的回答里也用它。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlatformHoldResponse {
    pub(crate) reason: String,
    pub(crate) held_at_unix_ms: i64,
}

impl PlatformHoldResponse {
    pub(crate) fn new(reason: String, held_at: UtcMillis) -> Self {
        Self {
            reason,
            held_at_unix_ms: held_at.value(),
        }
    }
}

async fn hold(
    State(state): State<PublicRoomPlatformHoldHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(HoldBody { reason }) = serde_json::from_slice(&body) else {
        return invalid("public_room_hold.invalid", correlation_id);
    };
    let (actor, catalog_id) =
        match authorize(&state, &catalog, &headers, &jar, correlation_id).await {
            Ok(authorized) => authorized,
            Err(response) => return response,
        };
    match state
        .holds
        .hold(HoldPublicRoom {
            actor,
            catalog_id,
            reason,
        })
        .await
    {
        Ok(hold) => no_store(
            (
                StatusCode::OK,
                Json(PlatformHoldResponse::new(hold.reason, hold.held_at)),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn release(
    State(state): State<PublicRoomPlatformHoldHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, catalog_id) =
        match authorize(&state, &catalog, &headers, &jar, correlation_id).await {
            Ok(authorized) => authorized,
            Err(response) => return response,
        };
    match state
        .holds
        .release(ReleasePublicRoom { actor, catalog_id })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 来源对、最近登录过、房间号像样。
async fn authorize(
    state: &PublicRoomPlatformHoldHttpState,
    catalog: &str,
    headers: &HeaderMap,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<(AuthenticatedPrincipal, RoomCatalogId), Response> {
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "public_room_hold.invalid_origin",
                ErrorCategory::Authorization,
                "拿掉公开房间的请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    let actor = authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await?;
    let catalog_id = parse_uuid_v7(catalog)
        .map(RoomCatalogId::from_uuid)
        .map_err(|()| invalid("public_room_hold.invalid_resource", correlation_id))?;
    Ok((actor, catalog_id))
}

/// 错误码统一以 `public_room_hold.` 开头；要重新登录的和别的要最近登录的操作同一个码。
fn failure_response(failure: PlatformHoldFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        PlatformHoldFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "public_room_hold.invalid",
            ErrorCategory::Validation,
            "原因最多 500 个字，不能带换行以外的控制字符。",
        ),
        PlatformHoldFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "public_room_hold.forbidden",
            ErrorCategory::Authorization,
            "只有平台治理的人能拿掉公开房间，或者登录已失效。",
        ),
        PlatformHoldFailureKind::ReauthenticationRequired => (
            StatusCode::UNAUTHORIZED,
            "authentication.reauthentication_required",
            ErrorCategory::Authentication,
            "该操作需要近期重新认证。",
        ),
        PlatformHoldFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "public_room_hold.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者房间已经归档。",
        ),
        PlatformHoldFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "public_room_hold.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时没办成，请稍后再试一次。",
        ),
        PlatformHoldFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "public_room_hold.internal",
            ErrorCategory::Transient,
            "拿掉公开房间发生内部错误。",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "拿掉或放回公开房间的请求失败"
    );
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
