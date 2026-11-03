//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员给自己的
//! Agent 管理授权。
//!
//! - `GET /private-rooms/{c}/moderation-grants`：这个房间在用的授权（房主和管理员）。
//! - `POST /private-rooms/{c}/moderation-grants`：给一份，要最近登录过。正文
//!   `{"agentId", "rules", "actions": ["mute", "kick", "ban", "unban"], "lifetimeSeconds"?}`，不填时长是
//!   30 天。同一个 Agent 再给一次盖掉上一份。回 201 和这一份。
//! - `DELETE /private-rooms/{c}/moderation-grants/{id}`：撤掉，回 204；已经撤了的也回 204。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::ActiveRoomModerationGrant,
    private_rooms::{
        GrantRoomModeration, ListRoomModerationGrants, ModerationGrantFailure,
        ModerationGrantFailureKind, RevokeRoomModerationGrant, RoomModerationGrantUseCases,
    },
};
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId, RoomModerationGrantId},
    room_moderation_grants::RoomModerationPower,
    time::DurationMillis,
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
use uuid::Uuid;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

/// 规则最多 2000 个字，一个字最多 4 个字节，再留出 JSON 转义的余量。
const MAX_GRANT_BODY_BYTES: usize = 16 * 1_024;

#[derive(Clone)]
pub(crate) struct RoomModerationGrantHttpState {
    grants: Arc<dyn RoomModerationGrantUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
}

impl RoomModerationGrantHttpState {
    pub(crate) fn new(
        grants: Arc<dyn RoomModerationGrantUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            grants,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: RoomModerationGrantHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/moderation-grants",
            get(list).post(create),
        )
        .route(
            "/private-rooms/{catalog_id}/moderation-grants/{grant_id}",
            delete(revoke),
        )
        .layer(DefaultBodyLimit::max(MAX_GRANT_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantBody {
    agent_id: String,
    rules: String,
    actions: Vec<String>,
    #[serde(default)]
    lifetime_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrantResponse {
    id: String,
    agent_id: String,
    agent_name: String,
    /// 给的人的账户 ID，网页按成员名单换成昵称。
    granted_by: String,
    rules: String,
    actions: Vec<&'static str>,
    created_at_unix_ms: i64,
    expires_at_unix_ms: i64,
}

impl From<ActiveRoomModerationGrant> for GrantResponse {
    fn from(active: ActiveRoomModerationGrant) -> Self {
        let grant = active.grant;
        Self {
            id: grant.id.to_string(),
            agent_id: grant.agent_id.to_string(),
            agent_name: active.agent_display_name,
            granted_by: grant.granted_by.to_string(),
            rules: grant.rules.as_str().to_owned(),
            actions: grant
                .powers
                .as_slice()
                .iter()
                .map(|power| power.as_str())
                .collect(),
            created_at_unix_ms: grant.created_at.value(),
            expires_at_unix_ms: grant.expires_at.value(),
        }
    }
}

#[derive(Debug, Serialize)]
struct GrantsResponse {
    grants: Vec<GrantResponse>,
}

async fn list(
    State(state): State<RoomModerationGrantHttpState>,
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
        return invalid("room_moderation_grant.invalid_resource", correlation_id);
    };
    match state
        .grants
        .list(ListRoomModerationGrants { actor, catalog_id })
        .await
    {
        Ok(grants) => no_store(
            Json(GrantsResponse {
                grants: grants.into_iter().map(GrantResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn create(
    State(state): State<RoomModerationGrantHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(body) = serde_json::from_slice::<GrantBody>(&body) else {
        return invalid("room_moderation_grant.invalid", correlation_id);
    };
    if !origin_matches(&headers, &state.trusted_origins) {
        return invalid_origin(correlation_id);
    }
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(catalog_id) = parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid) else {
        return invalid("room_moderation_grant.invalid_resource", correlation_id);
    };
    let Some(request) = body.into_request(actor, catalog_id) else {
        return invalid("room_moderation_grant.invalid", correlation_id);
    };
    match state.grants.grant(request).await {
        Ok(grant) => {
            no_store((StatusCode::CREATED, Json(GrantResponse::from(grant))).into_response())
        }
        Err(failure) => failure_response(failure, correlation_id),
    }
}

impl GrantBody {
    /// 正文里的 ID、勾的几样和时长认得出来才往下交；规则和范围由用例层查。
    fn into_request(
        self,
        actor: agent_room_application::authentication::AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
    ) -> Option<GrantRoomModeration> {
        let agent_id = parse_uuid_v7(&self.agent_id).map(AgentId::from_uuid).ok()?;
        let powers = self
            .actions
            .iter()
            .map(|action| RoomModerationPower::try_from(action.as_str()).ok())
            .collect::<Option<Vec<_>>>()?;
        let lifetime = match self.lifetime_seconds {
            None => None,
            Some(seconds) => Some(DurationMillis::new(seconds.checked_mul(1_000)?).ok()?),
        };
        Some(GrantRoomModeration {
            actor,
            catalog_id,
            grant_id: RoomModerationGrantId::from_uuid(Uuid::now_v7()),
            agent_id,
            rules: self.rules,
            powers,
            lifetime,
        })
    }
}

async fn revoke(
    State(state): State<RoomModerationGrantHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, grant)): Path<(String, String)>,
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
    let (Ok(catalog_id), Ok(grant_id)) = (
        parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid),
        parse_uuid_v7(&grant).map(RoomModerationGrantId::from_uuid),
    ) else {
        return invalid("room_moderation_grant.invalid_resource", correlation_id);
    };
    match state
        .grants
        .revoke(RevokeRoomModerationGrant {
            actor,
            catalog_id,
            grant_id,
        })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `room_moderation_grant.` 开头，网页按它换成中英文的说法。
fn failure_response(failure: ModerationGrantFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        ModerationGrantFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "room_moderation_grant.invalid",
            ErrorCategory::Validation,
            "规则要写 1 到 2000 个字，至少勾一样，时长在一分钟到 30 天之间。",
        ),
        ModerationGrantFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_moderation_grant.forbidden",
            ErrorCategory::Authorization,
            "只有房主和管理员能把房间交给 Agent 管，或者登录已失效。",
        ),
        ModerationGrantFailureKind::NotYourAgent => (
            StatusCode::FORBIDDEN,
            "room_moderation_grant.not_your_agent",
            ErrorCategory::Authorization,
            "只能交给你自己的、还在用的 Agent。",
        ),
        ModerationGrantFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "room_moderation_grant.not_found",
            ErrorCategory::Validation,
            "没有这个房间、你不在房间里，或者没有这份授权。",
        ),
        ModerationGrantFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "room_moderation_grant.conflict",
            ErrorCategory::Conflict,
            "房间已经归档。",
        ),
        ModerationGrantFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_moderation_grant.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时没办成，请稍后再试。",
        ),
        ModerationGrantFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_moderation_grant.internal",
            ErrorCategory::Transient,
            "管理授权发生内部错误。",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "管理授权的请求失败"
    );
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn invalid_origin(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::FORBIDDEN,
            "room_moderation_grant.invalid_origin",
            ErrorCategory::Authorization,
            "管理授权的请求来源无效。",
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
