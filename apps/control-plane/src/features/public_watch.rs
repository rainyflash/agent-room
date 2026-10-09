//! 不登录也能看公开大厅：`GET /public-lobbies/{slug}/watch`（specs/public-lobby-watch/design.md）。
//!
//! 不要登录、不看 Cookie。回答是内存里的快照，3 秒内的重复请求浏览器可以直接用缓存。

use std::sync::Arc;

use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Router,
    extract::{Extension, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::no_store,
    public_watch::{PublicWatch, PublicWatchFailure},
};

/// 快照最多 3 秒重读一次，浏览器缓存同样久。
const CACHE_CONTROL: &str = "public, max-age=3";
/// 暂时读不到时，请网页隔这么多秒再问。
const RETRY_AFTER_SECONDS: u64 = 5;

#[derive(Clone)]
pub(crate) struct PublicWatchHttpState {
    pub(crate) watch: Arc<PublicWatch>,
}

pub(crate) fn router(state: PublicWatchHttpState) -> Router {
    Router::new()
        .route("/public-lobbies/{slug}/watch", get(watch))
        .with_state(state)
}

async fn watch(
    State(state): State<PublicWatchHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(slug): Path<String>,
) -> Response {
    match state.watch.watch(&slug).await {
        Ok(body) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static(CACHE_CONTROL),
                ),
            ],
            body,
        )
            .into_response(),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

fn failure_response(failure: PublicWatchFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure {
        PublicWatchFailure::Disabled => (
            StatusCode::NOT_FOUND,
            "public_watch.disabled",
            ErrorCategory::Validation,
            "这台服务器没有开放不登录看公开大厅。",
        ),
        PublicWatchFailure::LobbyNotFound => (
            StatusCode::NOT_FOUND,
            "public_watch.lobby_not_found",
            ErrorCategory::Validation,
            "没有这个公开大厅。",
        ),
        PublicWatchFailure::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "public_watch.unavailable",
            ErrorCategory::DependencyUnavailable,
            "公开大厅暂时看不了，请稍后再试。",
        ),
    };
    let mut error = ApiError::new(status, code, category, message, correlation_id);
    if failure == PublicWatchFailure::Unavailable {
        error = error.retry_after_seconds(RETRY_AFTER_SECONDS);
    }
    no_store(error.into_response())
}
