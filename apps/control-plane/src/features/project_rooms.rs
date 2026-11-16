//! 项目房间的认领和绑定（`specs/redesign/project-rooms.md` 的“接口”，3P-2b）。要登录，和别的私人房间
//! 接口一样在 API 域名上：
//!
//! - `POST /project-rooms/claims`（`{ repository, catalogId? }`）认领一个仓库，同一个人同一个仓库再来回
//!   同一个；`GET`、`DELETE /project-rooms/claims/{claimId}`；`POST .../check` 现在核对（每个认领每分钟
//!   一次，太快回 429 带 `Retry-After`）。
//! - `GET`、`DELETE /private-rooms/{c}/repository`（只有房主：绑着哪个仓库；解绑），
//!   `POST /private-rooms/{c}/repository/check`（再核对一次）。
//!
//! 回答里的 `shareUrl`、`agentsLine`（`AGENTS.md` 那一行）、`badgeMarkdown`（README 徽章那一行）都照部署的
//! 站点（`https://<裸域>`）拼好：网页、命令行都不知道裸域在哪。都是新接口，不往别的回答里加字段（网页
//! 对回答 `.strict()` 校验）。
//!
//! 这一步只认网页登录。本机 Bridge 替这台电脑认领（设备签名）随 3P-4a 加。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    private_rooms::PrivateRoomActor,
    project_rooms::{
        BindingCheckResult, BindingCheckView, BindingView, ClaimRepository, ClaimStatus, ClaimView,
        InspectClaim, InspectRoomRepository, ProjectRoomFailure, ProjectRoomFailureKind,
        ProjectRoomUseCases,
    },
};
use agent_room_domain::{
    ids::{RepositoryClaimId, RoomCatalogId},
    project_rooms::GithubRepository,
    room_listing::RoomShareCode,
    time::UtcMillis,
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
    error::{ApiError, seconds_until},
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
        share_page::{share_url, site_origin},
    },
};

const MAX_CLAIM_BODY_BYTES: usize = 1_024;

#[derive(Clone)]
pub(crate) struct ProjectRoomHttpState {
    rooms: Arc<dyn ProjectRoomUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    trusted_origins: TrustedOrigins,
    site_origin: Arc<str>,
}

impl ProjectRoomHttpState {
    pub(crate) fn new(
        rooms: Arc<dyn ProjectRoomUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
        server_name: &str,
    ) -> Self {
        Self {
            rooms,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
            site_origin: site_origin(server_name).into(),
        }
    }
}

pub(crate) fn router(state: ProjectRoomHttpState) -> Router {
    Router::new()
        .route("/project-rooms/claims", post(claim))
        .route(
            "/project-rooms/claims/{claim_id}",
            get(inspect_claim).delete(cancel_claim),
        )
        .route("/project-rooms/claims/{claim_id}/check", post(check_claim))
        .route(
            "/private-rooms/{catalog_id}/repository",
            get(inspect_binding).delete(unbind),
        )
        .route(
            "/private-rooms/{catalog_id}/repository/check",
            post(check_binding),
        )
        .layer(DefaultBodyLimit::max(MAX_CLAIM_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaimBody {
    repository: String,
    #[serde(default)]
    catalog_id: Option<String>,
}

/// `AGENTS.md` 那一行和 README 徽章，照设计里的样子拼。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryLines {
    share_url: String,
    agents_line: String,
    badge_markdown: String,
}

impl RepositoryLines {
    fn new(site_origin: &str, repository: &GithubRepository, share_code: &RoomShareCode) -> Self {
        let share_url = share_url(site_origin, share_code.as_str());
        Self {
            agents_line: format!(
                "Agent Room: this project's room is {share_url}. Agents join with `agent-room join {share_url}`, or over the network via {site_origin}/agents.txt"
            ),
            badge_markdown: format!(
                "[![Agent Room]({site_origin}/gh/{}/{}/badge.svg)]({share_url})",
                repository.owner(),
                repository.name()
            ),
            share_url,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResponse {
    at_unix_ms: i64,
    result: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaimResponse {
    /// 认上以后认领就没了，是 `null`。
    claim_id: Option<String>,
    repository: String,
    share_code: String,
    #[serde(flatten)]
    lines: RepositoryLines,
    status: &'static str,
    /// 要绑的那间房；认上以后是绑上的那间房。认领了再建的、还在等的是 `null`。
    catalog_id: Option<String>,
    claimed_at_unix_ms: Option<i64>,
    last_check: Option<CheckResponse>,
}

impl ClaimResponse {
    fn new(view: &ClaimView, site_origin: &str) -> Self {
        Self {
            claim_id: view.claim_id.map(|id| id.as_uuid().to_string()),
            repository: view.repository.full_name(),
            share_code: view.share_code.as_str().to_owned(),
            lines: RepositoryLines::new(site_origin, &view.repository, &view.share_code),
            status: match view.status {
                ClaimStatus::Pending => "pending",
                ClaimStatus::Bound => "bound",
            },
            catalog_id: view.catalog_id.map(|id| id.as_uuid().to_string()),
            claimed_at_unix_ms: view.claimed_at.map(UtcMillis::value),
            last_check: view.last_check.map(|check| CheckResponse {
                at_unix_ms: check.at.value(),
                result: check.result.as_str(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BindingResponse {
    repository: String,
    repository_url: String,
    verified_by_account_id: String,
    verified_at_unix_ms: i64,
    verified_file: &'static str,
    /// 绑着仓库的房间都公开着、有分享码；万一没有，这几样是 `null`。
    share_code: Option<String>,
    share_url: Option<String>,
    agents_line: Option<String>,
    badge_markdown: Option<String>,
}

impl BindingResponse {
    fn new(view: BindingView, site_origin: &str) -> Self {
        let binding = view.binding;
        let lines = view
            .share_code
            .as_ref()
            .map(|code| RepositoryLines::new(site_origin, &binding.repository, code));
        Self {
            repository_url: format!("https://github.com/{}", binding.repository.full_name()),
            repository: binding.repository.full_name(),
            verified_by_account_id: binding.verified_by.as_uuid().to_string(),
            verified_at_unix_ms: binding.verified_at.value(),
            verified_file: binding.verified_file.as_str(),
            share_code: view.share_code.map(|code| code.as_str().to_owned()),
            share_url: lines.as_ref().map(|lines| lines.share_url.clone()),
            agents_line: lines.as_ref().map(|lines| lines.agents_line.clone()),
            badge_markdown: lines.map(|lines| lines.badge_markdown),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BindingCheckResponse {
    binding: BindingResponse,
    last_check: CheckResponse,
}

impl BindingCheckResponse {
    fn new(view: BindingCheckView, site_origin: &str) -> Self {
        Self {
            binding: BindingResponse::new(view.binding, site_origin),
            last_check: CheckResponse {
                at_unix_ms: view.checked_at.value(),
                result: match view.result {
                    BindingCheckResult::Found(_) => "found",
                    BindingCheckResult::Missing(result) => result.as_str(),
                },
            },
        }
    }
}

async fn claim(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let actor = match changing_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<ClaimBody>(&body) else {
        return invalid("project_room.invalid_request", correlation_id);
    };
    let target = match body.catalog_id.as_deref().map(catalog_id) {
        None => None,
        Some(Some(catalog_id)) => Some(catalog_id),
        Some(None) => return invalid("project_room.invalid_request", correlation_id),
    };
    let result = state
        .rooms
        .claim(ClaimRepository {
            actor,
            repository: body.repository,
            target,
        })
        .await;
    respond(result, correlation_id, |view| {
        ClaimResponse::new(&view, &state.site_origin)
    })
}

async fn inspect_claim(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(claim): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(claim_id) = claim_id(&claim) else {
        return invalid("project_room.invalid_resource", correlation_id);
    };
    let actor = match session(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let result = state
        .rooms
        .inspect_claim(InspectClaim { actor, claim_id })
        .await;
    respond(result, correlation_id, |view| {
        ClaimResponse::new(&view, &state.site_origin)
    })
}

async fn check_claim(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(claim): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let actor = match changing_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(claim_id) = claim_id(&claim) else {
        return invalid("project_room.invalid_resource", correlation_id);
    };
    let result = state
        .rooms
        .check_claim(InspectClaim { actor, claim_id })
        .await;
    respond(result, correlation_id, |view| {
        ClaimResponse::new(&view, &state.site_origin)
    })
}

async fn cancel_claim(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(claim): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let actor = match changing_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(claim_id) = claim_id(&claim) else {
        return invalid("project_room.invalid_resource", correlation_id);
    };
    match state
        .rooms
        .cancel_claim(InspectClaim { actor, claim_id })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn inspect_binding(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("project_room.invalid_resource", correlation_id);
    };
    let actor = match session(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let result = state
        .rooms
        .binding(InspectRoomRepository { actor, catalog_id })
        .await;
    respond(result, correlation_id, |view| {
        view.map(|view| BindingResponse::new(view, &state.site_origin))
    })
}

async fn unbind(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let actor = match changing_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("project_room.invalid_resource", correlation_id);
    };
    match state
        .rooms
        .unbind(InspectRoomRepository { actor, catalog_id })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

async fn check_binding(
    State(state): State<ProjectRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let actor = match changing_actor(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("project_room.invalid_resource", correlation_id);
    };
    let result = state
        .rooms
        .check_binding(InspectRoomRepository { actor, catalog_id })
        .await;
    respond(result, correlation_id, |view| {
        BindingCheckResponse::new(view, &state.site_origin)
    })
}

/// 要改东西的请求：先看来源是不是自己的网页（防跨站），再认登录。
async fn changing_actor(
    state: &ProjectRoomHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<PrivateRoomActor, Response> {
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "project_room.invalid_origin",
                ErrorCategory::Authorization,
                "认领仓库的请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    session(state, jar, correlation_id).await
}

async fn session(
    state: &ProjectRoomHttpState,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<PrivateRoomActor, Response> {
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    .map(PrivateRoomActor::Person)
}

fn respond<T, R: Serialize>(
    result: Result<T, ProjectRoomFailure>,
    correlation_id: CorrelationId,
    render: impl FnOnce(T) -> R,
) -> Response {
    match result {
        Ok(view) => no_store((StatusCode::OK, Json(render(view))).into_response()),
        Err(failure) => failure_response(failure, correlation_id),
    }
}

/// 错误码以 `project_room.` 开头，网页、命令行按它换成自己的说法；限额和换码沿用公开房间的两个码。
fn failure_response(failure: ProjectRoomFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure.kind() {
        ProjectRoomFailureKind::InvalidRepository => (
            StatusCode::BAD_REQUEST,
            "project_room.invalid_repository",
            ErrorCategory::Validation,
            "仓库要写成 owner/repo，名字照 GitHub 的规矩。",
        ),
        ProjectRoomFailureKind::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "project_room.invalid_request",
            ErrorCategory::Validation,
            "认领仓库的请求无效。",
        ),
        ProjectRoomFailureKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "project_room.forbidden",
            ErrorCategory::Authorization,
            "只有房主能用这间房认领、看和改它绑的仓库，或者登录已失效。",
        ),
        ProjectRoomFailureKind::NotFound => (
            StatusCode::NOT_FOUND,
            "project_room.not_found",
            ErrorCategory::Validation,
            "没有这个认领或房间，或者房间没绑仓库。",
        ),
        ProjectRoomFailureKind::TooManyClaims => (
            StatusCode::CONFLICT,
            "project_room.too_many_claims",
            ErrorCategory::Conflict,
            "你已经有 3 个认领在等了，先认上或者撤掉一个。",
        ),
        ProjectRoomFailureKind::RepositoryTaken => (
            StatusCode::CONFLICT,
            "project_room.repository_taken",
            ErrorCategory::Conflict,
            "这个仓库已经绑着一间房，现在认领不了。",
        ),
        ProjectRoomFailureKind::RoomNotEligible => (
            StatusCode::CONFLICT,
            "project_room.room_not_eligible",
            ErrorCategory::Conflict,
            "只能绑你名下公开着的房间；房间改回私密、关了、换了分享码以后，认领作废。",
        ),
        ProjectRoomFailureKind::RoomAlreadyBound => (
            StatusCode::CONFLICT,
            "room_listing.repository_bound",
            ErrorCategory::Conflict,
            "这间房已经绑着别的仓库，先解绑。",
        ),
        ProjectRoomFailureKind::CheckTooSoon => (
            StatusCode::TOO_MANY_REQUESTS,
            "project_room.check_too_soon",
            ErrorCategory::Transient,
            "每分钟只能核对一次，过一会儿再点。",
        ),
        ProjectRoomFailureKind::LimitReached => (
            StatusCode::CONFLICT,
            "private_room.public_limit_reached",
            ErrorCategory::Conflict,
            "不绑仓库的公开房间每个账户只能有一间，先把另一间改回私密。",
        ),
        ProjectRoomFailureKind::Conflict => (
            StatusCode::CONFLICT,
            "project_room.conflict",
            ErrorCategory::Conflict,
            "这间房同时还有别人的认领在等，请稍后再试。",
        ),
        ProjectRoomFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "project_room.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时没做成，请稍后再试。",
        ),
        ProjectRoomFailureKind::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "project_room.internal",
            ErrorCategory::Transient,
            "认领仓库发生内部错误。",
        ),
    };
    tracing::info!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "项目房间请求失败"
    );
    let error = ApiError::new(status, code, category, message, correlation_id);
    let error = match failure.retry_at() {
        Some(retry_at) => error.retry_after_seconds(seconds_until(retry_at)),
        None => error,
    };
    no_store(error.into_response())
}

fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}

fn claim_id(value: &str) -> Option<RepositoryClaimId> {
    parse_uuid_v7(value).map(RepositoryClaimId::from_uuid).ok()
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

#[cfg(test)]
mod tests;
