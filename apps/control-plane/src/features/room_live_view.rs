//! 分享页放实况的开关（`specs/redesign/project-rooms.md`“分享页放实况”“接口”，3P-6a）：
//! `GET /private-rooms/{c}/live-view` 和 `PUT /private-rooms/{c}/live-view`（正文 `{"enabled": true}`）。
//!
//! 房间里已加入的人都读得到：开没开、什么时候开的、此刻放没放、差哪几样、自己能不能开关。只有房主和
//! 管理员能开关。不满足条件时开不了，回 409 `room_live.not_eligible`，`details.condition` 是哪一样
//! （`sub_room`、`closed`、`not_listed`、`platform_hold`、`encrypted`、`join_mode`）。
//!
//! 是新的接口，不往公开设置、房间的回答里加字段：网页对那些 `.strict()` 校验，旧版桌面端多看到一个
//! 字段整页都读不出来。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    room_live::{
        InspectLiveView, LiveViewCondition, LiveViewFailure, LiveViewFailureKind, LiveViewState,
        RoomLiveViewUseCases, UpdateLiveView,
    },
};
use agent_room_domain::ids::RoomCatalogId;
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
        resource_ids::parse_uuid_v7,
    },
};

/// 正文只有一个开关。
const MAX_BODY_BYTES: usize = 1_024;

#[derive(Clone)]
pub(crate) struct RoomLiveViewHttpState {
    live_views: Arc<dyn RoomLiveViewUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl RoomLiveViewHttpState {
    pub(crate) fn new(
        live_views: Arc<dyn RoomLiveViewUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            live_views,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: RoomLiveViewHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/live-view",
            get(inspect).put(update),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveViewBody {
    enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveViewResponse {
    /// 房主或管理员开着。
    enabled: bool,
    /// 开的那一刻，只放这以后说的话；关着时为空。
    enabled_at_unix_ms: Option<i64>,
    /// 分享页此刻放着：开着，而且几样都满足。
    live: bool,
    /// 此刻不满足的几样，按检查的先后。
    unmet_conditions: Vec<&'static str>,
    /// 问的人能不能开关。
    can_change: bool,
}

impl From<LiveViewState> for LiveViewResponse {
    fn from(state: LiveViewState) -> Self {
        Self {
            live: state.live(),
            enabled: state.enabled.is_some(),
            enabled_at_unix_ms: state.enabled.map(|record| record.enabled_at.value()),
            unmet_conditions: state
                .unmet
                .iter()
                .map(|condition| condition.as_str())
                .collect(),
            can_change: state.can_change,
        }
    }
}

async fn inspect(
    State(state): State<RoomLiveViewHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
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
        return invalid("room_live.invalid_resource", correlation_id);
    };
    respond(
        state
            .live_views
            .inspect(InspectLiveView { actor, catalog_id })
            .await,
        correlation_id,
    )
}

async fn update(
    State(state): State<RoomLiveViewHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(LiveViewBody { enabled }) = serde_json::from_slice(&body) else {
        return invalid("room_live.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_live.invalid_origin",
                ErrorCategory::Authorization,
                "开关实况的请求来源无效。",
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
        return invalid("room_live.invalid_resource", correlation_id);
    };
    respond(
        state
            .live_views
            .update(UpdateLiveView {
                actor,
                catalog_id,
                enabled,
            })
            .await,
        correlation_id,
    )
}

fn respond(
    result: Result<LiveViewState, LiveViewFailure>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(state) => {
            no_store((StatusCode::OK, Json(LiveViewResponse::from(state))).into_response())
        }
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_live.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: LiveViewFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        LiveViewFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_live.forbidden",
            ErrorCategory::Authorization,
            "只有房主和管理员能开关实况，或者登录已失效。",
        ),
        LiveViewFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_live.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在房间里。",
        ),
        LiveViewFailureKind::NotEligible(condition) => {
            return not_eligible(failure, condition, correlation_id);
        }
        LiveViewFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_live.conflict",
            ErrorCategory::Conflict,
            "房间的设置刚被改过，请刷新以后再开。",
        ),
        LiveViewFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_live.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "实况暂时没开关成，请稍后再点一次。",
        ),
        LiveViewFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_live.internal",
            ErrorCategory::Transient,
            "开关实况发生内部错误。",
        ),
    };
    log_failure(failure, correlation_id);
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn not_eligible(
    failure: LiveViewFailure,
    condition: LiveViewCondition,
    correlation_id: CorrelationId,
) -> Response {
    let message = match condition {
        LiveViewCondition::SubRoom => "子房间不能放实况，只有主房间能放。",
        LiveViewCondition::Closed => "房间已经关了，不能放实况。",
        LiveViewCondition::NotListed => "房间不公开，不能放实况：先设成公开。",
        LiveViewCondition::PlatformHold => "这间房被平台从目录里拿掉了，不能放实况。",
        LiveViewCondition::Encrypted => "加密的房间不能放实况。",
        LiveViewCondition::JoinMode => "只有直接进的房间能放实况，要申请、只能邀请的房间不能放。",
    };
    log_failure(failure, correlation_id);
    no_store(
        ApiError::new(
            StatusCode::CONFLICT,
            "room_live.not_eligible",
            ErrorCategory::Conflict,
            message,
            correlation_id,
        )
        .with_detail("condition", serde_json::Value::from(condition.as_str()))
        .into_response(),
    )
}

fn log_failure(failure: LiveViewFailure, correlation_id: CorrelationId) {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "开关实况的请求失败"
    );
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
