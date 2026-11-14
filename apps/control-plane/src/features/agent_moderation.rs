//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：本机 Agent 照主人给的管理授权
//! 动手。设备签名的两条和 Agent 离开房间那条一个样子，Bridge 用；网络 Agent 拿不到授权，没有它的入口。
//!
//! - `GET /agents/{a}/instances/{i}/rooms/{c}/moderation`：它在这个房间里的授权（规则、能做哪几样、到什么
//!   时候），没有就回 404 `agent_moderation.not_granted`。
//! - `POST /agents/{a}/instances/{i}/rooms/{c}/moderation/actions`：动手，`Idempotency-Key` 是动作 ID（UUID v7）。
//!   正文 `{"action": "mute"|"kick"|"ban"|"unban", "target": "<发言人的聊天账号>", "reason"?, "note",
//!   "durationSeconds"?}`，回 201。解封不用 `reason`、回的是它撤回的那条封禁；只有禁言收时长。
//! - `GET /rooms/{c}/moderation/actions/agents`：房主和管理员看台账时，哪些是 Agent 做的、哪些是 Agent 撤回
//!   的（解封）、它写的理由。单独一个接口：网页对治理动作严格校验（`moderationActionSchema`），动作里多一个
//!   字段，旧版的台账整个读不出来。
//! - `POST /rooms/{c}/moderation/grants/{g}/reverse`：房主和管理员把这份授权下 Agent 做的、还生效着的动作
//!   一次全撤，要最近登录，正文 `{"impactAcknowledged": true}`。回 200
//!   `{"reversed": ["<动作 ID>"], "failed": [{"actionId", "code"}]}`，一条没撤成不挡别的，再来一次只会再试没撤成的。
//!
//! 给 Agent 看的错误说明是英文，和两份 MCP 的说法一样。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    devices::DeviceAuthorizationUseCases,
    moderation::{
        AgentModerate, AgentModerationFailure, AgentModerationFailureKind, AgentModerationScope,
        AgentModerationUseCases, AgentModerationView, GrantReversal, ListAgentModeration,
        ModerationFailureKind, ReverseGrantModeration,
    },
    ports::{AgentModerationRecord, MatrixUserId, SecretFactory},
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, ModerationActionId, RoomCatalogId, RoomModerationGrantId},
    moderation::{ModerationAction, ModerationReason},
    room_moderation_grants::{AgentModerationNote, RoomModerationPower},
    time::{DurationMillis, UtcMillis},
};
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
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

/// 理由最多 500 个字，一个字最多 4 个字节，再留出 JSON 转义和别的字段的余量。
const MAX_ACTION_BODY_BYTES: usize = 8 * 1_024;
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

#[derive(Clone)]
pub(crate) struct AgentModerationHttpState {
    moderation: Arc<dyn AgentModerationUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct AgentModerationHttpDependencies {
    pub(crate) moderation: Arc<dyn AgentModerationUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
    /// 一次全撤是网页发的写请求，和人工撤回一样只认这些来源。
    pub(crate) trusted_origins: TrustedOrigins,
}

impl AgentModerationHttpState {
    pub(crate) fn new(dependencies: AgentModerationHttpDependencies) -> Self {
        Self {
            moderation: dependencies.moderation,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: dependencies.trusted_origins,
        }
    }
}

pub(crate) fn router(state: AgentModerationHttpState) -> Router {
    Router::new()
        .route(
            "/agents/{agent_id}/instances/{instance_id}/rooms/{catalog_id}/moderation",
            get(rules),
        )
        .route(
            "/agents/{agent_id}/instances/{instance_id}/rooms/{catalog_id}/moderation/actions",
            post(moderate),
        )
        .route(
            "/rooms/{catalog_id}/moderation/actions/agents",
            get(list_agent_actions),
        )
        .route(
            "/rooms/{catalog_id}/moderation/grants/{grant_id}/reverse",
            post(reverse_grant),
        )
        .layer(DefaultBodyLimit::max(MAX_ACTION_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RulesResponse {
    grant_id: String,
    rules: String,
    actions: Vec<&'static str>,
    expires_at_unix_ms: i64,
}

impl From<AgentModerationView> for RulesResponse {
    fn from(view: AgentModerationView) -> Self {
        Self {
            grant_id: view.grant_id.to_string(),
            rules: view.rules.as_str().to_owned(),
            actions: view
                .powers
                .as_slice()
                .iter()
                .map(|power| power.as_str())
                .collect(),
            expires_at_unix_ms: view.expires_at.value(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActionBody {
    action: String,
    target: String,
    #[serde(default)]
    reason: Option<String>,
    note: String,
    #[serde(default)]
    duration_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionResponse {
    action_id: String,
    action: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at_unix_ms: Option<i64>,
}

impl From<ModerationAction> for ActionResponse {
    fn from(action: ModerationAction) -> Self {
        Self {
            action_id: action.id().to_string(),
            action: action.kind().as_str(),
            status: action.status().as_str(),
            expires_at_unix_ms: action.expires_at().map(UtcMillis::value),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentActionResponse {
    action_id: String,
    agent_id: String,
    agent_name: String,
    grant_id: String,
    note: String,
}

impl From<AgentModerationRecord> for AgentActionResponse {
    fn from(record: AgentModerationRecord) -> Self {
        let attribution = record.attribution;
        Self {
            action_id: attribution.action_id.to_string(),
            agent_id: attribution.agent_id.to_string(),
            agent_name: record.agent_display_name,
            grant_id: attribution.grant_id.to_string(),
            note: attribution.note.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Serialize)]
struct AgentActionsResponse {
    actions: Vec<AgentActionResponse>,
    /// Agent 撤回的动作（解封）：`actionId` 是被撤回的那条。
    reversals: Vec<AgentActionResponse>,
}

/// 路径里的 Agent、连接和房间，都是 UUID v7。
fn scope_ids(
    (agent, instance, catalog): &(String, String, String),
) -> Option<(AgentId, AgentInstanceId, RoomCatalogId)> {
    Some((
        AgentId::from_uuid(parse_uuid_v7(agent).ok()?),
        AgentInstanceId::from_uuid(parse_uuid_v7(instance).ok()?),
        RoomCatalogId::from_uuid(parse_uuid_v7(catalog).ok()?),
    ))
}

async fn rules(
    State(state): State<AgentModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(path): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let request_target = format!(
        "/agents/{}/instances/{}/rooms/{}/moderation",
        path.0, path.1, path.2
    );
    let Some((agent_id, agent_instance_id, room_catalog_id)) = scope_ids(&path) else {
        return invalid("agent_moderation.invalid_resource", correlation_id);
    };
    let device = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "GET",
        &request_target,
        "",
        correlation_id,
    )
    .await
    {
        Ok(device) => device,
        Err(response) => return response,
    };
    let scope = AgentModerationScope {
        device,
        agent_id,
        agent_instance_id,
        room_catalog_id,
    };
    match state.moderation.rules(scope).await {
        Ok(view) => no_store(Json(RulesResponse::from(view)).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn moderate(
    State(state): State<AgentModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(path): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!(
        "/agents/{}/instances/{}/rooms/{}/moderation/actions",
        path.0, path.1, path.2
    );
    let Some((agent_id, agent_instance_id, room_catalog_id)) = scope_ids(&path) else {
        return invalid("agent_moderation.invalid_resource", correlation_id);
    };
    let Some(action_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_uuid_v7(value).ok())
        .map(ModerationActionId::from_uuid)
    else {
        return invalid("agent_moderation.invalid_idempotency_key", correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid("agent_moderation.invalid", correlation_id);
    };
    let device = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "POST",
        &request_target,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(device) => device,
        Err(response) => return response,
    };
    let scope = AgentModerationScope {
        device,
        agent_id,
        agent_instance_id,
        room_catalog_id,
    };
    let Some(request) = serde_json::from_slice::<ActionBody>(&body)
        .ok()
        .and_then(|body| body.into_request(scope, action_id))
    else {
        return invalid("agent_moderation.invalid", correlation_id);
    };
    match state.moderation.moderate(request).await {
        Ok(action) => {
            no_store((StatusCode::CREATED, Json(ActionResponse::from(action))).into_response())
        }
        Err(failure) => failure_response(failure, correlation_id),
    }
}

impl ActionBody {
    /// 能做的一样、聊天账号、理由的类别、写的理由和时长认得出来才往下交；授权和对象由用例层查。
    fn into_request(
        self,
        scope: AgentModerationScope,
        action_id: ModerationActionId,
    ) -> Option<AgentModerate> {
        let duration = match self.duration_seconds {
            None => None,
            Some(seconds) => Some(DurationMillis::new(seconds.checked_mul(1_000)?).ok()?),
        };
        Some(AgentModerate {
            scope,
            action_id,
            power: RoomModerationPower::try_from(self.action.as_str()).ok()?,
            target: MatrixUserId::new(self.target.trim()).ok()?,
            reason: match self.reason {
                None => None,
                Some(reason) => Some(ModerationReason::try_from(reason.as_str()).ok()?),
            },
            note: AgentModerationNote::parse(&self.note).ok()?,
            duration,
        })
    }
}

async fn list_agent_actions(
    State(state): State<AgentModerationHttpState>,
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
    let Ok(room_catalog_id) = parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid) else {
        return invalid("agent_moderation.invalid_resource", correlation_id);
    };
    match state
        .moderation
        .list(ListAgentModeration {
            actor,
            room_catalog_id,
        })
        .await
    {
        Ok(ledger) => no_store(
            Json(AgentActionsResponse {
                actions: ledger
                    .actions
                    .into_iter()
                    .map(AgentActionResponse::from)
                    .collect(),
                reversals: ledger
                    .reversals
                    .into_iter()
                    .map(AgentActionResponse::from)
                    .collect(),
            })
            .into_response(),
        ),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReverseGrantBody {
    impact_acknowledged: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailedReversal {
    action_id: String,
    code: &'static str,
}

#[derive(Debug, Serialize)]
struct GrantReversalResponse {
    reversed: Vec<String>,
    failed: Vec<FailedReversal>,
}

impl From<GrantReversal> for GrantReversalResponse {
    fn from(outcome: GrantReversal) -> Self {
        Self {
            reversed: outcome
                .reversed
                .iter()
                .map(|action| action.id().to_string())
                .collect(),
            failed: outcome
                .failed
                .into_iter()
                .map(|(action_id, kind)| FailedReversal {
                    action_id: action_id.to_string(),
                    code: ledger_code(kind),
                })
                .collect(),
        }
    }
}

/// 和人工撤回回的错误码一样（`ApiError::moderation`）。
const fn ledger_code(kind: ModerationFailureKind) -> &'static str {
    match kind {
        ModerationFailureKind::InvalidRequest => "moderation.invalid_request",
        ModerationFailureKind::Forbidden => "moderation.forbidden",
        ModerationFailureKind::OwnerOnly => "private_room.owner_only",
        ModerationFailureKind::NotFound => "moderation.not_found",
        ModerationFailureKind::Conflict => "moderation.conflict",
        ModerationFailureKind::RateLimited => "moderation.rate_limited",
        ModerationFailureKind::DependencyUnavailable => "moderation.dependency_unavailable",
        ModerationFailureKind::Internal => "moderation.internal",
    }
}

async fn reverse_grant(
    State(state): State<AgentModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, grant)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "agent_moderation.invalid_origin",
                ErrorCategory::Authorization,
                "This request did not come from an Agent Room page.",
                correlation_id,
            )
            .into_response(),
        );
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
    let (Ok(room_catalog_id), Ok(grant_id)) = (
        parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid),
        parse_uuid_v7(&grant).map(RoomModerationGrantId::from_uuid),
    ) else {
        return invalid("agent_moderation.invalid_resource", correlation_id);
    };
    let Ok(body) = serde_json::from_slice::<ReverseGrantBody>(&body) else {
        return invalid("agent_moderation.invalid", correlation_id);
    };
    match state
        .moderation
        .reverse_grant(ReverseGrantModeration {
            actor,
            room_catalog_id,
            grant_id,
            impact_acknowledged: body.impact_acknowledged,
        })
        .await
    {
        Ok(outcome) => no_store(Json(GrantReversalResponse::from(outcome)).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码统一以 `agent_moderation.` 开头。说明是给 Agent 看的，写英文。
fn failure_response(failure: AgentModerationFailure, correlation_id: CorrelationId) -> Response {
    use AgentModerationFailureKind as Kind;
    let (status, code, category, message) = match failure.kind() {
        Kind::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "agent_moderation.unauthorized",
            ErrorCategory::Authentication,
            "This connection does not belong to that agent, or its sign-in has expired.",
        ),
        Kind::NotGranted => (
            StatusCode::NOT_FOUND,
            "agent_moderation.not_granted",
            ErrorCategory::Authorization,
            "You have no moderation grant in this room, or it was revoked or has expired.",
        ),
        Kind::NotAllowed => (
            StatusCode::FORBIDDEN,
            "agent_moderation.not_allowed",
            ErrorCategory::Authorization,
            "Your grant in this room does not include this action.",
        ),
        Kind::Protected => (
            StatusCode::FORBIDDEN,
            "agent_moderation.protected",
            ErrorCategory::Authorization,
            "You cannot act on the room owner, its admins, the person who granted you, or their agents.",
        ),
        Kind::TargetNotFound => (
            StatusCode::NOT_FOUND,
            "agent_moderation.target_not_found",
            ErrorCategory::Validation,
            "No person or agent uses that chat account. Pass the sender's Matrix user ID from the message.",
        ),
        Kind::NotInRoom => (
            StatusCode::CONFLICT,
            "agent_moderation.not_in_room",
            ErrorCategory::Conflict,
            "That person is no longer in this room.",
        ),
        Kind::NotBanned => (
            StatusCode::CONFLICT,
            "agent_moderation.not_banned",
            ErrorCategory::Conflict,
            "That person is not banned from this room.",
        ),
        Kind::BannedByPerson => (
            StatusCode::FORBIDDEN,
            "agent_moderation.banned_by_person",
            ErrorCategory::Authorization,
            "A person banned them. Only the room owner or an admin can lift that ban.",
        ),
        Kind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "agent_moderation.invalid",
            ErrorCategory::Validation,
            "Check the action, the target, a reason (not needed to unban), a note of 1 to 500 characters, and a duration between 1 minute and 30 days (mute only).",
        ),
        Kind::Forbidden => (
            StatusCode::FORBIDDEN,
            "agent_moderation.forbidden",
            ErrorCategory::Authorization,
            "Only the room owner and admins can see this.",
        ),
        Kind::NotFound => (
            StatusCode::NOT_FOUND,
            "agent_moderation.not_found",
            ErrorCategory::Validation,
            "The room is gone or archived, or that person is no longer active.",
        ),
        Kind::Conflict => (
            StatusCode::CONFLICT,
            "agent_moderation.conflict",
            ErrorCategory::Conflict,
            "That action ID was already used for a different action.",
        ),
        Kind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_moderation.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "The chat server did not take the action. Try again shortly.",
        ),
        Kind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "agent_moderation.internal",
            ErrorCategory::Transient,
            "Moderation failed with an internal error.",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "Agent 管房间的请求失败"
    );
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
