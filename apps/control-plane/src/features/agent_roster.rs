use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};
use agent_room_application::{
    agent_roster::AgentRosterService,
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
};
use agent_room_domain::{agent_lifecycle::AgentRosterPolicy, ids::RoomCatalogId};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::put,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct AgentRosterHttpState {
    pub service: Arc<AgentRosterService>,
    pub authentication: Arc<dyn AuthenticationUseCases>,
    pub trusted_origins: TrustedOrigins,
}

pub(crate) fn router(state: AgentRosterHttpState) -> Router {
    Router::new()
        .route("/rooms/{catalog_id}/agent-roster-policy", put(update))
        .layer(DefaultBodyLimit::max(1_024))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyBody {
    archive_after_days: u16,
}

async fn update(
    State(state): State<AgentRosterHttpState>,
    Extension(id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<PolicyBody>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "agent_roster.invalid_origin",
                ErrorCategory::Authorization,
                "请求来源无效。",
                id,
            )
            .into_response(),
        );
    }
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(catalog_id) = parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid) else {
        return no_store(
            ApiError::invalid_request("agent_roster.invalid_catalog", id).into_response(),
        );
    };
    let policy = match body {
        Ok(Json(body)) => AgentRosterPolicy::new(body.archive_after_days),
        Err(_) => None,
    };
    let Some(policy) = policy else {
        return no_store(
            ApiError::invalid_request("agent_roster.invalid_policy", id).into_response(),
        );
    };
    match state.service.update(actor, catalog_id, policy).await {
        Ok(()) => no_store(Json(serde_json::json!({ "schemaVersion": 1, "archiveAfterDays": policy.archive_after_days() })).into_response()),
        Err(error) => no_store(ApiError::moderation(error, id).into_response()),
    }
}
