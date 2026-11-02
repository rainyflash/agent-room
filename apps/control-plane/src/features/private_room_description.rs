//! 房主改房间简介（`specs/public-rooms/design.md`）：`PUT /private-rooms/{c}/description`，正文
//! `{"description": "..."}`，空的就是去掉简介。回答改好以后的简介（去掉了首尾空白）。
//!
//! 简介本来就在私人房间的回答里，改完网页照旧读房间，不用另加字段。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{
        DescribePrivateRoom, DescriptionFailure, DescriptionFailureKind,
        PrivateRoomDescriptionUseCases,
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

/// 简介最多 2048 个字，一个字最多 4 个字节，再留出 JSON 转义的余量。
const MAX_DESCRIPTION_BODY_BYTES: usize = 16 * 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomDescriptionHttpState {
    descriptions: Arc<dyn PrivateRoomDescriptionUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl PrivateRoomDescriptionHttpState {
    pub(crate) fn new(
        descriptions: Arc<dyn PrivateRoomDescriptionUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            descriptions,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PrivateRoomDescriptionHttpState) -> Router {
    Router::new()
        .route("/private-rooms/{catalog_id}/description", put(describe))
        .layer(DefaultBodyLimit::max(MAX_DESCRIPTION_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptionBody {
    description: String,
}

#[derive(Debug, Serialize)]
struct DescriptionResponse {
    description: String,
}

async fn describe(
    State(state): State<PrivateRoomDescriptionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(DescriptionBody { description }) = serde_json::from_slice(&body) else {
        return invalid("room_description.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_description.invalid_origin",
                ErrorCategory::Authorization,
                "改简介的请求来源无效。",
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
        return invalid("room_description.invalid_resource", correlation_id);
    };
    match state
        .descriptions
        .describe(DescribePrivateRoom {
            actor,
            catalog_id,
            description,
        })
        .await
    {
        Ok(description) => {
            no_store((StatusCode::OK, Json(DescriptionResponse { description })).into_response())
        }
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_description.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: DescriptionFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        DescriptionFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "room_description.invalid",
            ErrorCategory::Validation,
            "简介最多 2048 个字，不能带控制字符。",
        ),
        DescriptionFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_description.forbidden",
            ErrorCategory::Authorization,
            "只有房主能改简介，或者登录已失效。",
        ),
        DescriptionFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_description.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在房间里。",
        ),
        DescriptionFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_description.conflict",
            ErrorCategory::Conflict,
            "房间已经归档，不能再改简介。",
        ),
        DescriptionFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_description.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "简介暂时没改成，请稍后再点一次。",
        ),
        DescriptionFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_description.internal",
            ErrorCategory::Transient,
            "改简介发生内部错误。",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "改简介的请求失败"
    );
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
