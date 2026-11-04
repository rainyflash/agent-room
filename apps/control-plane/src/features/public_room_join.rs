//! 进公开房间（`specs/public-rooms/design.md`）：
//!
//! - `POST /private-rooms/{c}/join`：直接进。以房主的名义邀请他，回 `invited`，网页接着照接受
//!   邀请那一套在 Matrix 上加入、确认；已经在里面的回 `joined`。
//! - `POST /private-rooms/{c}/join-requests`：申请，可以留一句话；`DELETE` 撤回自己的。
//! - `GET /private-rooms/{c}/join-requests`：房主和管理员看在等的申请；
//!   `PUT .../join-requests/{principalId}` 批准或拒绝。
//!
//! 不公开的、没有的房间回同一个 404。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationUseCases},
    ports::PendingJoinRequest,
    private_rooms::{
        AnswerJoinRequest, JoinPublicRoom, JoinRequestState, PublicJoinFailure,
        PublicJoinFailureKind, PublicJoinMembership, PublicRoomJoinUseCases, RequestToJoin,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_join_requests::JoinRequestStatus,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{post, put},
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

/// 申请留言最多 200 字，正文再大也是乱发的。
const MAX_JOIN_BODY_BYTES: usize = 4 * 1_024;

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
        .route(
            "/private-rooms/{catalog_id}/join-requests",
            post(request).delete(withdraw).get(pending),
        )
        .route(
            "/private-rooms/{catalog_id}/join-requests/{principal_id}",
            put(answer),
        )
        .layer(DefaultBodyLimit::max(MAX_JOIN_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinResponse {
    catalog_id: String,
    /// `pending`（在等批准）、`invited` 或 `joined`。
    membership: &'static str,
    /// 在等批准时，什么时候申请的。
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_at_unix_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestBody {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnswerBody {
    decision: Decision,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Approve,
    Reject,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingResponse {
    requests: Vec<PendingRequestResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingRequestResponse {
    principal_id: String,
    display_name: String,
    message: String,
    requested_at_unix_ms: i64,
}

impl From<PendingJoinRequest> for PendingRequestResponse {
    fn from(pending: PendingJoinRequest) -> Self {
        Self {
            principal_id: pending.record.requester.to_string(),
            display_name: pending.requester_display_name,
            message: pending.record.message.as_str().to_owned(),
            requested_at_unix_ms: pending.record.requested_at.value(),
        }
    }
}

async fn join(
    State(state): State<PublicRoomJoinHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, catalog_id) = match changing(&state, &headers, &jar, &catalog, correlation_id).await
    {
        Ok(caller) => caller,
        Err(response) => return response,
    };
    match state.joins.join(JoinPublicRoom { actor, catalog_id }).await {
        Ok(membership) => joined(catalog_id, JoinRequestState::Member(membership)),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn request(
    State(state): State<PublicRoomJoinHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let body = if body.is_empty() {
        RequestBody::default()
    } else {
        match serde_json::from_slice::<RequestBody>(&body) {
            Ok(body) => body,
            Err(_) => return invalid("room_join.invalid", correlation_id),
        }
    };
    let (actor, catalog_id) = match changing(&state, &headers, &jar, &catalog, correlation_id).await
    {
        Ok(caller) => caller,
        Err(response) => return response,
    };
    match state
        .joins
        .request(RequestToJoin {
            actor,
            catalog_id,
            message: body.message,
        })
        .await
    {
        Ok(state) => joined(catalog_id, state),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn withdraw(
    State(state): State<PublicRoomJoinHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, catalog_id) = match changing(&state, &headers, &jar, &catalog, correlation_id).await
    {
        Ok(caller) => caller,
        Err(response) => return response,
    };
    match state
        .joins
        .withdraw(JoinPublicRoom { actor, catalog_id })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn pending(
    State(state): State<PublicRoomJoinHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("room_join.invalid_resource", correlation_id);
    };
    let actor = match session(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .joins
        .pending_requests(JoinPublicRoom { actor, catalog_id })
        .await
    {
        Ok(requests) => no_store(
            (
                StatusCode::OK,
                Json(PendingResponse {
                    requests: requests.into_iter().map(Into::into).collect(),
                }),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn answer(
    State(state): State<PublicRoomJoinHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, principal)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(body) = serde_json::from_slice::<AnswerBody>(&body) else {
        return invalid("room_join.invalid", correlation_id);
    };
    let Ok(requester) = parse_uuid_v7(&principal).map(PrincipalId::from_uuid) else {
        return invalid("room_join.invalid_resource", correlation_id);
    };
    let (actor, catalog_id) = match changing(&state, &headers, &jar, &catalog, correlation_id).await
    {
        Ok(caller) => caller,
        Err(response) => return response,
    };
    let approve = matches!(body.decision, Decision::Approve);
    match state
        .joins
        .answer(AnswerJoinRequest {
            actor,
            catalog_id,
            requester,
            approve,
        })
        .await
    {
        Ok(status) => no_store(
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": match status {
                        JoinRequestStatus::Approved => "approved",
                        _ => "rejected",
                    }
                })),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

fn joined(catalog_id: RoomCatalogId, state: JoinRequestState) -> Response {
    let (membership, requested_at_unix_ms) = match state {
        JoinRequestState::Pending { requested_at } => ("pending", Some(requested_at.value())),
        JoinRequestState::Member(PublicJoinMembership::Invited) => ("invited", None),
        JoinRequestState::Member(PublicJoinMembership::Joined) => ("joined", None),
    };
    no_store(
        (
            StatusCode::OK,
            Json(JoinResponse {
                catalog_id: catalog_id.to_string(),
                membership,
                requested_at_unix_ms,
            }),
        )
            .into_response(),
    )
}

/// 改东西的请求：来自自己的网页、登录有效、房间号对。
async fn changing(
    state: &PublicRoomJoinHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    catalog: &str,
    correlation_id: CorrelationId,
) -> Result<(AuthenticatedPrincipal, RoomCatalogId), Response> {
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "room_join.invalid_origin",
                ErrorCategory::Authorization,
                "进房间的请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    let actor = session(state, jar, correlation_id).await?;
    let catalog_id =
        catalog_id(catalog).ok_or_else(|| invalid("room_join.invalid_resource", correlation_id))?;
    Ok((actor, catalog_id))
}

async fn session(
    state: &PublicRoomJoinHttpState,
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

fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

/// 错误码统一以 `room_join.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: PublicJoinFailure, correlation_id: CorrelationId) -> Response {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "进公开房间的请求失败"
    );
    let (status, code, category, message) = match failure.kind() {
        PublicJoinFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "room_join.invalid",
            ErrorCategory::Validation,
            "申请留言最多 200 字，只能换行。",
        ),
        PublicJoinFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_join.forbidden",
            ErrorCategory::Authorization,
            "登录已失效，或者你不能处理这个房间的申请。",
        ),
        PublicJoinFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_join.not_found",
            ErrorCategory::Validation,
            "这个房间现在不公开了，或者没有这个申请。",
        ),
        PublicJoinFailureKind::Banned => (
            StatusCode::FORBIDDEN,
            "room_join.banned",
            ErrorCategory::Authorization,
            "被这个房间封禁了，进不来。",
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
        PublicJoinFailureKind::RecentlyRejected => {
            let error = ApiError::new(
                StatusCode::CONFLICT,
                "room_join.recently_rejected",
                ErrorCategory::Conflict,
                "申请被拒绝了，24 小时以后才能再申请。",
                correlation_id,
            );
            let error = match failure.retry_at() {
                Some(retry_at) => error.retry_after_seconds(seconds_until(retry_at)),
                None => error,
            };
            return no_store(error.into_response());
        }
        PublicJoinFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_join.conflict",
            ErrorCategory::Conflict,
            "房间或申请刚刚变了，请再试一次。",
        ),
        PublicJoinFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_join.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时没办成，请稍后再点一次。",
        ),
        PublicJoinFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_join.internal",
            ErrorCategory::Transient,
            "进房间发生内部错误。",
        ),
    };
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
