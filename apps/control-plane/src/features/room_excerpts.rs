//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”“接口”，3P-7a）：
//!
//! - `POST /private-rooms/{c}/excerpts`，正文 `{"firstEventId": "$…", "lastEventId": "$…"}`，回 201
//!   `{ code, url }`。放着实况的房间里已加入的人都能分享；只认开了实况以后、主房间里连着的最多 30 句。
//! - `GET /private-rooms/{c}/excerpts`：自己分享的，房主和管理员看这间房的全部，回
//!   `{ excerpts: [{ code, url, messageCount, firstMessageAtUnixMs, sharedAtUnixMs, mine }] }`，不写是谁
//!   分享的。
//! - `DELETE /private-rooms/{c}/excerpts/{code}`：分享的人自己、房主和管理员，回 204。
//!
//! 错误码都以 `room_excerpt.` 开头，说法是中文，网页按错误码换中英文的说法（3P-7b）：`not_live`（409，房间
//! 此刻没在放实况）、`too_long`（422，超过 30 句）、`invalid_range`（422，`details.reason` 是 `not_found`
//! 或 `before_live`）、`not_found`、`forbidden`、`rate_limited`（429，带 `Retry-After`）、
//! `dependency_unavailable`、`internal`、`invalid`、`invalid_origin`、`invalid_resource`。
//!
//! 都是新的接口和新的回答，不往房间、公开设置、实况开关的回答里加字段：网页对那些 `.strict()` 校验。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::MatrixEventId,
    room_excerpts::{
        DeleteExcerpt, ExcerptFailure, ExcerptFailureKind, ExcerptList, ExcerptRangeProblem,
        ListExcerpts, RoomExcerptUseCases, ShareExcerpt,
    },
};
use agent_room_domain::{
    ids::RoomCatalogId, room_excerpts::RoomExcerptCode, room_listing::RoomShareCode,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
        share_page::{excerpt_url, site_origin},
    },
};

/// 正文只有两个事件 ID，每个最长 255 字节。
const MAX_BODY_BYTES: usize = 2_048;

#[derive(Clone)]
pub(crate) struct RoomExcerptHttpState {
    excerpts: Arc<dyn RoomExcerptUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
    /// 分享链接所在的网站（裸域）。
    site_origin: Arc<str>,
}

impl RoomExcerptHttpState {
    pub(crate) fn new(
        excerpts: Arc<dyn RoomExcerptUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        server_name: &str,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            excerpts,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
            site_origin: site_origin(server_name).into(),
        }
    }

    fn url(&self, share_code: &RoomShareCode, code: &RoomExcerptCode) -> String {
        excerpt_url(&self.site_origin, share_code.as_str(), code.as_str())
    }
}

pub(crate) fn router(state: RoomExcerptHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/excerpts",
            get(list).post(share),
        )
        .route(
            "/private-rooms/{catalog_id}/excerpts/{code}",
            delete(remove),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ShareBody {
    first_event_id: String,
    last_event_id: String,
}

#[derive(Debug, Serialize)]
struct SharedResponse {
    code: String,
    url: String,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    excerpts: Vec<ExcerptView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExcerptView {
    code: String,
    /// 房间没有分享码时为空（不会有：放实况的房间都公开着）。
    url: Option<String>,
    message_count: usize,
    first_message_at_unix_ms: i64,
    shared_at_unix_ms: i64,
    /// 是不是问的人自己分享的。别人分享的不写是谁。
    mine: bool,
}

async fn share(
    State(state): State<RoomExcerptHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(ShareBody {
        first_event_id,
        last_event_id,
    }) = serde_json::from_slice(&body)
    else {
        return invalid("room_excerpt.invalid", correlation_id);
    };
    let (Ok(first_event_id), Ok(last_event_id)) = (
        MatrixEventId::new(first_event_id),
        MatrixEventId::new(last_event_id),
    ) else {
        return invalid("room_excerpt.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return invalid_origin(correlation_id);
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
        return invalid("room_excerpt.invalid_resource", correlation_id);
    };
    match state
        .excerpts
        .share(ShareExcerpt {
            actor,
            catalog_id,
            first_event_id,
            last_event_id,
        })
        .await
    {
        Ok(shared) => no_store(
            (
                StatusCode::CREATED,
                Json(SharedResponse {
                    url: state.url(&shared.share_code, &shared.code),
                    code: shared.code.as_str().to_owned(),
                }),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn list(
    State(state): State<RoomExcerptHttpState>,
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
        return invalid("room_excerpt.invalid_resource", correlation_id);
    };
    match state
        .excerpts
        .list(ListExcerpts { actor, catalog_id })
        .await
    {
        Ok(listing) => no_store((StatusCode::OK, Json(list_view(&state, listing))).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn remove(
    State(state): State<RoomExcerptHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, code)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return invalid_origin(correlation_id);
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
    let (Ok(catalog_id), Ok(code)) = (
        parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid),
        RoomExcerptCode::parse(&code),
    ) else {
        return invalid("room_excerpt.invalid_resource", correlation_id);
    };
    match state
        .excerpts
        .delete(DeleteExcerpt {
            actor,
            catalog_id,
            code,
        })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

fn list_view(state: &RoomExcerptHttpState, listing: ExcerptList) -> ListResponse {
    ListResponse {
        excerpts: listing
            .excerpts
            .into_iter()
            .map(|excerpt| ExcerptView {
                url: listing
                    .share_code
                    .as_ref()
                    .map(|share_code| state.url(share_code, &excerpt.code)),
                code: excerpt.code.as_str().to_owned(),
                message_count: excerpt.message_count,
                first_message_at_unix_ms: excerpt.first_message_at.value(),
                shared_at_unix_ms: excerpt.shared_at.value(),
                mine: excerpt.mine,
            })
            .collect(),
    }
}

/// 错误码统一以 `room_excerpt.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: ExcerptFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        ExcerptFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_excerpt.forbidden",
            ErrorCategory::Authorization,
            "只有分享的人、房主和管理员能删这一段，或者登录已失效。",
        ),
        ExcerptFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_excerpt.not_found",
            ErrorCategory::Validation,
            "没有这个房间或者这一段，或者你不在房间里。",
        ),
        ExcerptFailureKind::NotLive => (
            StatusCode::CONFLICT,
            "room_excerpt.not_live",
            ErrorCategory::Conflict,
            "这间房的分享页现在没在放实况，不能分享一段对话。",
        ),
        ExcerptFailureKind::TooLong => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "room_excerpt.too_long",
            ErrorCategory::Validation,
            "一段最多 30 句。",
        ),
        ExcerptFailureKind::InvalidRange(problem) => {
            return invalid_range(failure, problem, correlation_id);
        }
        ExcerptFailureKind::RateLimited {
            retry_after_seconds,
        } => {
            log_failure(failure, correlation_id);
            return no_store(
                ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "room_excerpt.rate_limited",
                    ErrorCategory::Transient,
                    "这一小时分享得太多了，过一会儿再分享。",
                    correlation_id,
                )
                .retry_after_seconds(retry_after_seconds)
                .into_response(),
            );
        }
        ExcerptFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_excerpt.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时没分享成，请稍后再试。",
        ),
        ExcerptFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_excerpt.internal",
            ErrorCategory::Transient,
            "分享一段对话发生内部错误。",
        ),
    };
    log_failure(failure, correlation_id);
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn invalid_range(
    failure: ExcerptFailure,
    problem: ExcerptRangeProblem,
    correlation_id: CorrelationId,
) -> Response {
    let message = match problem {
        ExcerptRangeProblem::NotFound => {
            "最近的消息里找不到这几句：可能太早了、撤回或隐藏了，或者结尾在开头前面。"
        }
        ExcerptRangeProblem::BeforeLive => "只能分享开了实况以后说的话。",
    };
    log_failure(failure, correlation_id);
    no_store(
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "room_excerpt.invalid_range",
            ErrorCategory::Validation,
            message,
            correlation_id,
        )
        .with_detail("reason", serde_json::Value::from(problem.as_str()))
        .into_response(),
    )
}

fn log_failure(failure: ExcerptFailure, correlation_id: CorrelationId) {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "分享一段对话的请求失败"
    );
}

fn invalid_origin(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::FORBIDDEN,
            "room_excerpt.invalid_origin",
            ErrorCategory::Authorization,
            "分享一段对话的请求来源无效。",
            correlation_id,
        )
        .into_response(),
    )
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
