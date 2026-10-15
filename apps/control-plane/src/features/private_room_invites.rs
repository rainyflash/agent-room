//! 私人房间的邀请链接（`specs/room-people/design.md`）：有邀请权限的成员在网页上生成、再看、停用；
//! 拿到链接的人登录后先看一眼是哪个房间、谁邀请的，再用它进来。
//!
//! 链接里的秘密在 `/join#` 后面，浏览器不发给服务器；网页读出来以后放在请求正文里。都是新的接口，
//! 不往私人房间的回答里加字段（网页对它 `.strict()` 校验）。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::{
        CreateInviteLink, InviteLink, InviteLinkFailure, InviteLinkFailureKind, InvitePreview,
        InviteeMembership, ManageInviteLink, PrivateRoomInviteLinkUseCases, RedeemedInvite,
        UseInviteLink,
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
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::{ApiError, seconds_until},
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        private_rooms::{PermissionsBody, PermissionsResponse},
        resource_ids::parse_uuid_v7,
    },
};

const MAX_INVITE_LINK_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomInviteHttpState {
    links: Arc<dyn PrivateRoomInviteLinkUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
    /// 链接写成网页的地址：桌面端页面的 Origin 不是网页的，拼不出别人能打开的链接。
    join_page: String,
}

impl PrivateRoomInviteHttpState {
    pub(crate) fn new(
        links: Arc<dyn PrivateRoomInviteLinkUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            links,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
            join_page: format!("{}/join", frontend_origin.as_str().trim_end_matches('/')),
        }
    }
}

pub(crate) fn router(state: PrivateRoomInviteHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/invite-link",
            get(reveal).post(create).delete(disable),
        )
        .route("/private-room-invites/preview", post(preview))
        .route("/private-room-invites/redeem", post(redeem))
        .layer(DefaultBodyLimit::max(MAX_INVITE_LINK_BODY_BYTES))
        .with_state(state)
}

/// 生成链接的正文可以不带：给默认的权限。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateBody {
    #[serde(default)]
    permissions: Option<PermissionsBody>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SecretBody {
    secret: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteLinkResponse {
    /// 可以直接发给别人的链接：`<网页>/join#<秘密>`。
    url: String,
    secret: String,
    permissions: PermissionsResponse,
    created_at_unix_ms: i64,
    expires_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewResponse {
    catalog_id: String,
    room_name: String,
    inviter_display_name: String,
    permissions: PermissionsResponse,
    expires_at_unix_ms: i64,
    membership: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RedeemResponse {
    catalog_id: String,
    membership: &'static str,
}

impl InviteLinkResponse {
    fn new(link: &InviteLink, join_page: &str) -> Self {
        Self {
            url: format!("{join_page}#{}", link.secret.as_str()),
            secret: link.secret.as_str().to_owned(),
            permissions: PermissionsResponse::from(link.record.permissions),
            created_at_unix_ms: link.record.created_at.value(),
            expires_at_unix_ms: link.record.expires_at.value(),
        }
    }
}

impl From<InvitePreview> for PreviewResponse {
    fn from(preview: InvitePreview) -> Self {
        Self {
            catalog_id: preview.catalog_id.to_string(),
            room_name: preview.room_name,
            inviter_display_name: preview.inviter_display_name,
            permissions: PermissionsResponse::from(preview.permissions),
            expires_at_unix_ms: preview.expires_at.value(),
            membership: membership(preview.membership),
        }
    }
}

impl From<RedeemedInvite> for RedeemResponse {
    fn from(redeemed: RedeemedInvite) -> Self {
        Self {
            catalog_id: redeemed.catalog_id.to_string(),
            membership: membership(redeemed.membership),
        }
    }
}

const fn membership(membership: InviteeMembership) -> &'static str {
    match membership {
        InviteeMembership::Absent => "absent",
        InviteeMembership::Invited => "invited",
        InviteeMembership::Joined => "joined",
    }
}

async fn create(
    State(state): State<PrivateRoomInviteHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Some(body) = parse_create_body(&body) else {
        return invalid("invite_link.invalid_permissions", correlation_id);
    };
    let permissions = match body.permissions.map(PermissionsBody::into_domain) {
        None => None,
        Some(Some(permissions)) => Some(permissions),
        Some(None) => return invalid("invite_link.invalid_permissions", correlation_id),
    };
    let actor = match write_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid_resource(correlation_id);
    };
    link_response(
        &state,
        state
            .links
            .create(CreateInviteLink {
                actor,
                catalog_id,
                permissions,
            })
            .await,
        correlation_id,
    )
}

/// 有邀请权限的人再看房间当前的链接。没有、过期了或者不管用了回答 `invite_link.not_found`。
async fn reveal(
    State(state): State<PrivateRoomInviteHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid_resource(correlation_id);
    };
    let actor = match session(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    link_response(
        &state,
        state
            .links
            .reveal(ManageInviteLink { actor, catalog_id })
            .await,
        correlation_id,
    )
}

async fn disable(
    State(state): State<PrivateRoomInviteHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let actor = match write_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid_resource(correlation_id);
    };
    match state
        .links
        .disable(ManageInviteLink { actor, catalog_id })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn preview(
    State(state): State<PrivateRoomInviteHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let request = match use_request(&state, &headers, &jar, &body, correlation_id).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    match state.links.preview(request).await {
        Ok(preview) => {
            no_store((StatusCode::OK, Json(PreviewResponse::from(preview))).into_response())
        }
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 用链接进来：回答房间和他现在的处境，网页接着在 Matrix 上加入、接受邀请。
async fn redeem(
    State(state): State<PrivateRoomInviteHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let request = match use_request(&state, &headers, &jar, &body, correlation_id).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    match state.links.redeem(request).await {
        Ok(redeemed) => {
            no_store((StatusCode::OK, Json(RedeemResponse::from(redeemed))).into_response())
        }
        Err(failure) => failure_response(failure, correlation_id),
    }
}

fn parse_create_body(body: &[u8]) -> Option<CreateBody> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Some(CreateBody::default());
    }
    serde_json::from_slice(body).ok()
}

async fn use_request(
    state: &PrivateRoomInviteHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    body: &[u8],
    correlation_id: CorrelationId,
) -> Result<UseInviteLink, Response> {
    let Ok(SecretBody { secret }) = serde_json::from_slice(body) else {
        return Err(invalid("invite_link.invalid", correlation_id));
    };
    let actor = write_actor(state, headers, jar, correlation_id).await?;
    Ok(UseInviteLink { actor, secret })
}

/// 写请求和带着秘密的请求都要来自自己的网页或桌面端，再要有效的登录。
async fn write_actor(
    state: &PrivateRoomInviteHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "invite_link.invalid_origin",
                ErrorCategory::Authorization,
                "邀请链接的请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    session(state, jar, correlation_id).await
}

async fn session(
    state: &PrivateRoomInviteHttpState,
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

fn link_response(
    state: &PrivateRoomInviteHttpState,
    result: Result<InviteLink, InviteLinkFailure>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(link) => no_store(
            (
                StatusCode::OK,
                Json(InviteLinkResponse::new(&link, &state.join_page)),
            )
                .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `invite_link.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: InviteLinkFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        InviteLinkFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "invite_link.invalid",
            ErrorCategory::Validation,
            "链接不完整，请把整条链接复制过来。",
        ),
        InviteLinkFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "invite_link.forbidden",
            ErrorCategory::Authorization,
            "没有邀请权限，给不出这些权限，或者登录已失效。",
        ),
        InviteLinkFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "invite_link.not_found",
            ErrorCategory::Validation,
            "链接失效了，找邀请你的人要个新的。",
        ),
        InviteLinkFailureKind::Banned => (
            StatusCode::FORBIDDEN,
            "invite_link.banned",
            ErrorCategory::Authorization,
            "你被这个房间封禁了，进不来。",
        ),
        InviteLinkFailureKind::NotRevealable => (
            StatusCode::NOT_FOUND,
            "invite_link.not_revealable",
            ErrorCategory::Validation,
            "这条链接看不到了：这台服务器没法保管链接。要邀请人就换一条新的。",
        ),
        InviteLinkFailureKind::RateLimited => {
            log_failure(failure, correlation_id);
            let error = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "invite_link.rate_limited",
                ErrorCategory::Transient,
                "试错的链接太多了，请稍后再试。",
                correlation_id,
            );
            let error = match failure.retry_at() {
                Some(retry_at) => error.retry_after_seconds(seconds_until(retry_at)),
                None => error,
            };
            return no_store(error.into_response());
        }
        InviteLinkFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "invite_link.conflict",
            ErrorCategory::Conflict,
            "房间的状态刚刚变了，请再试一次。",
        ),
        InviteLinkFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "invite_link.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "邀请链接暂时用不了，请稍后再试。",
        ),
        InviteLinkFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invite_link.internal",
            ErrorCategory::Transient,
            "邀请链接发生内部错误。",
        ),
    };
    log_failure(failure, correlation_id);
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn log_failure(failure: InviteLinkFailure, correlation_id: CorrelationId) {
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "邀请链接请求失败"
    );
}

fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

fn invalid_resource(correlation_id: CorrelationId) -> Response {
    invalid("invite_link.invalid_resource", correlation_id)
}

#[cfg(test)]
mod tests;
