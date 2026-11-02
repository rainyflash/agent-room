//! 进公开房间（`specs/public-rooms/design.md`）：`POST /private-rooms/{c}/join`。直接进的房间以房主的
//! 名义邀请他，回 `invited`，网页接着照接受邀请那一套在 Matrix 上加入、确认；已经在里面的回
//! `joined`。不公开的、没有的房间回同一个 404。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{
        JoinPublicRoom, PublicJoinFailure, PublicJoinFailureKind, PublicJoinMembership,
        PublicRoomJoinUseCases,
    },
};
use agent_room_domain::ids::RoomCatalogId;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

/// 进房间不带正文。
const MAX_JOIN_BODY_BYTES: usize = 1_024;

#[derive(Clone)]
pub(crate) struct PublicRoomJoinHttpState {
    joins: Arc<dyn PublicRoomJoinUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl PublicRoomJoinHttpState {
    pub(crate) fn new(
        joins: Arc<dyn PublicRoomJoinUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            joins,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PublicRoomJoinHttpState) -> Router {
    Router::new()
        .route("/private-rooms/{catalog_id}/join", post(join))
        .layer(DefaultBodyLimit::max(MAX_JOIN_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinResponse {
    catalog_id: String,
    membership: &'static str,
}

async fn join(
    State(state): State<PublicRoomJoinHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_join.invalid_origin",
                ErrorCategory::Authorization,
                "进房间的请求来源无效。",
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
        return no_store(
            ApiError::invalid_request("room_join.invalid_resource", correlation_id).into_response(),
        );
    };
    match state.joins.join(JoinPublicRoom { actor, catalog_id }).await {
        Ok(membership) => no_store(
            (
                StatusCode::OK,
                Json(JoinResponse {
                    catalog_id: catalog_id.to_string(),
                    membership: match membership {
                        PublicJoinMembership::Invited => "invited",
                        PublicJoinMembership::Joined => "joined",
                    },
                }),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_join.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: PublicJoinFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        PublicJoinFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_join.forbidden",
            ErrorCategory::Authorization,
            "登录已失效，请重新登录。",
        ),
        PublicJoinFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_join.not_found",
            ErrorCategory::Validation,
            "这个房间现在不公开了。",
        ),
        PublicJoinFailureKind::Banned => (
            StatusCode::FORBIDDEN,
            "room_join.banned",
            ErrorCategory::Authorization,
            "你被这个房间封禁了。",
        ),
        PublicJoinFailureKind::ApprovalRequired => (
            StatusCode::CONFLICT,
            "room_join.approval_required",
            ErrorCategory::Conflict,
            "这个房间要申请，管理员批准以后才能进来。",
        ),
        PublicJoinFailureKind::InviteOnly => (
            StatusCode::FORBIDDEN,
            "room_join.invite_only",
            ErrorCategory::Authorization,
            "这个房间只能由房主邀请。",
        ),
        PublicJoinFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_join.conflict",
            ErrorCategory::Conflict,
            "房间刚刚变了，请再试一次。",
        ),
        PublicJoinFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_join.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时进不去，请稍后再点一次。",
        ),
        PublicJoinFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_join.internal",
            ErrorCategory::Transient,
            "进房间发生内部错误。",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "进公开房间失败"
    );
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
