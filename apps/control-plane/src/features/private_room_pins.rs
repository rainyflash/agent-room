//! 房主和管理员置顶房间里的一句、取消置顶（`specs/redesign/brief-and-pins.md`“置顶”）：
//! `PUT /private-rooms/{c}/pins`，正文 `{"eventId": "$...", "pinned": true | false}`，回改完的整份列表
//! `{"pinned": [{"eventId", "pinnedBy", "pinnedAtUnixMs"}]}`，新的在最前面。
//!
//! 没有读的接口：置顶是 Matrix 的房间状态（`m.room.pinned_events`），网页和 Agent 都从房间状态里读。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{PinsFailure, PinsFailureKind, PrivateRoomPinsUseCases, UpdateRoomPins},
};
use agent_room_domain::ids::RoomCatalogId;
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
    error::{ApiError, seconds_until},
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

/// 一个事件 ID 最长 255 字节，再留出 JSON 的余量。
const MAX_PINS_BODY_BYTES: usize = 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomPinsHttpState {
    pins: Arc<dyn PrivateRoomPinsUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl PrivateRoomPinsHttpState {
    pub(crate) fn new(
        pins: Arc<dyn PrivateRoomPinsUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            pins,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PrivateRoomPinsHttpState) -> Router {
    Router::new()
        .route("/private-rooms/{catalog_id}/pins", put(update_pins))
        .layer(DefaultBodyLimit::max(MAX_PINS_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PinsBody {
    event_id: String,
    pinned: bool,
}

#[derive(Debug, Serialize)]
struct PinsResponse {
    pinned: Vec<PinResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinResponse {
    event_id: String,
    pinned_by: String,
    pinned_at_unix_ms: i64,
}

async fn update_pins(
    State(state): State<PrivateRoomPinsHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(PinsBody { event_id, pinned }) = serde_json::from_slice(&body) else {
        return invalid("room_pins.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_pins.invalid_origin",
                ErrorCategory::Authorization,
                "改置顶的请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
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
    let Ok(catalog_id) = parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid) else {
        return invalid("room_pins.invalid_resource", correlation_id);
    };
    match state
        .pins
        .update_pins(UpdateRoomPins {
            actor,
            catalog_id,
            event_id,
            pinned,
        })
        .await
    {
        Ok(pins) => no_store(
            (
                StatusCode::OK,
                Json(PinsResponse {
                    pinned: pins
                        .into_iter()
                        .map(|pin| PinResponse {
                            event_id: pin.event_id,
                            pinned_by: pin.pinned_by,
                            pinned_at_unix_ms: pin.pinned_at.value(),
                        })
                        .collect(),
                }),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_pins.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: PinsFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        PinsFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "room_pins.invalid",
            ErrorCategory::Validation,
            "要置顶的是一条 Matrix 事件。",
        ),
        PinsFailureKind::InvalidEvent => (
            StatusCode::BAD_REQUEST,
            "room_pins.not_a_message",
            ErrorCategory::Validation,
            "只能置顶这个房间里还在的一句话。",
        ),
        PinsFailureKind::Full => (
            StatusCode::CONFLICT,
            "room_pins.full",
            ErrorCategory::Conflict,
            "已经置顶了 5 条，先取消一条再置顶。",
        ),
        PinsFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_pins.forbidden",
            ErrorCategory::Authorization,
            "只有房主和管理员能置顶，或者登录已失效。",
        ),
        PinsFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_pins.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在房间里。",
        ),
        PinsFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_pins.conflict",
            ErrorCategory::Conflict,
            "房间已经归档，不能再改置顶。",
        ),
        PinsFailureKind::RateLimited { retry_at } => {
            return rate_limited(failure, retry_at, correlation_id);
        }
        PinsFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_pins.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "置顶暂时没改成，请稍后再点一次。",
        ),
        PinsFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_pins.internal",
            ErrorCategory::Transient,
            "改置顶发生内部错误。",
        ),
    };
    log_failure(failure, correlation_id);
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn rate_limited(
    failure: PinsFailure,
    retry_at: agent_room_domain::time::UtcMillis,
    correlation_id: CorrelationId,
) -> Response {
    log_failure(failure, correlation_id);
    no_store(
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "room_pins.rate_limited",
            ErrorCategory::Transient,
            "这一小时改须知和置顶的次数太多了，请按 Retry-After 等一会儿再改。",
            correlation_id,
        )
        .retry_after_seconds(seconds_until(retry_at))
        .into_response(),
    )
}

fn log_failure(failure: PinsFailure, correlation_id: CorrelationId) {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "改置顶的请求失败"
    );
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
