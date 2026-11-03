//! 房主和管理员改房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：
//! `PUT /private-rooms/{c}/notice`，正文 `{"text": "..."}`，空的就是撤掉。回答写进去的那一版
//! （去掉了首尾空白）和改的时间。
//!
//! 没有读的接口：公告是 Matrix 的房间状态，网页和 Agent 都从房间状态里读。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{
        NoticeFailure, NoticeFailureKind, PrivateRoomNoticeUseCases, UpdateRoomNotice,
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
    error::{ApiError, seconds_until},
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

/// 公告最多 2000 个字，一个字最多 4 个字节，再留出 JSON 转义的余量。
const MAX_NOTICE_BODY_BYTES: usize = 16 * 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomNoticeHttpState {
    notices: Arc<dyn PrivateRoomNoticeUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl PrivateRoomNoticeHttpState {
    pub(crate) fn new(
        notices: Arc<dyn PrivateRoomNoticeUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            notices,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PrivateRoomNoticeHttpState) -> Router {
    Router::new()
        .route("/private-rooms/{catalog_id}/notice", put(update_notice))
        .layer(DefaultBodyLimit::max(MAX_NOTICE_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeBody {
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NoticeResponse {
    text: String,
    updated_at_unix_ms: i64,
}

async fn update_notice(
    State(state): State<PrivateRoomNoticeHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(NoticeBody { text }) = serde_json::from_slice(&body) else {
        return invalid("room_notice.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_notice.invalid_origin",
                ErrorCategory::Authorization,
                "改公告的请求来源无效。",
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
        return invalid("room_notice.invalid_resource", correlation_id);
    };
    match state
        .notices
        .update_notice(UpdateRoomNotice {
            actor,
            catalog_id,
            text,
        })
        .await
    {
        Ok(notice) => no_store(
            (
                StatusCode::OK,
                Json(NoticeResponse {
                    text: notice.text,
                    updated_at_unix_ms: notice.updated_at.value(),
                }),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_notice.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: NoticeFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        NoticeFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "room_notice.invalid",
            ErrorCategory::Validation,
            "公告最多 2000 个字，只能带换行和制表符这两种控制字符。",
        ),
        NoticeFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_notice.forbidden",
            ErrorCategory::Authorization,
            "只有房主和管理员能改公告，或者登录已失效。",
        ),
        NoticeFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_notice.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在房间里。",
        ),
        NoticeFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_notice.conflict",
            ErrorCategory::Conflict,
            "房间已经归档，不能再改公告。",
        ),
        NoticeFailureKind::RateLimited { retry_at } => {
            return rate_limited(failure, retry_at, correlation_id);
        }
        NoticeFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_notice.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "公告暂时没改成，请稍后再点一次。",
        ),
        NoticeFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_notice.internal",
            ErrorCategory::Transient,
            "改公告发生内部错误。",
        ),
    };
    log_failure(failure, correlation_id);
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn rate_limited(
    failure: NoticeFailure,
    retry_at: agent_room_domain::time::UtcMillis,
    correlation_id: CorrelationId,
) -> Response {
    log_failure(failure, correlation_id);
    no_store(
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "room_notice.rate_limited",
            ErrorCategory::Transient,
            "这一小时改公告的次数太多了，请按 Retry-After 等一会儿再改。",
            correlation_id,
        )
        .retry_after_seconds(seconds_until(retry_at))
        .into_response(),
    )
}

fn log_failure(failure: NoticeFailure, correlation_id: CorrelationId) {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "改公告的请求失败"
    );
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
