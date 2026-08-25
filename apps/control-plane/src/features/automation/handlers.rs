use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement},
    automation::{ListAutomationGrants, RevokeAutomationGrant},
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json,
    body::Bytes,
    extract::{Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;

use super::{
    AutomationHttpState,
    models::{
        AuthorizeAutomationSendBody, AutomationAuthorizationResponse, AutomationGrantListResponse,
        AutomationGrantResponse, CreateAutomationGrantBody, grant_id,
    },
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
    },
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

pub(super) async fn create(
    State(state): State<AutomationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<CreateAutomationGrantBody>, JsonRejection>,
) -> Response {
    let actor = match authenticate_write(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(grant_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(grant_id)
    else {
        return invalid("automation.invalid_idempotency_key", correlation_id);
    };
    let Ok(Json(body)) = body else {
        return invalid("automation.invalid_creation_body", correlation_id);
    };
    let Some(request) = body.into_request(actor, grant_id) else {
        return invalid("automation.invalid_creation_body", correlation_id);
    };
    match state.automation.create(request).await {
        Ok(record) => no_store(
            (
                StatusCode::CREATED,
                Json(AutomationGrantResponse::from(record)),
            )
                .into_response(),
        ),
        Err(failure) => no_store(ApiError::automation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn list(
    State(state): State<AutomationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
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
    match state.automation.list(ListAutomationGrants { actor }).await {
        Ok(grants) => no_store(Json(AutomationGrantListResponse::from(grants)).into_response()),
        Err(failure) => no_store(ApiError::automation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn revoke(
    State(state): State<AutomationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(grant): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let Some(grant_id) = grant_id(&grant) else {
        return invalid("automation.invalid_grant_id", correlation_id);
    };
    let actor = match authenticate_write(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .automation
        .revoke(RevokeAutomationGrant { actor, grant_id })
        .await
    {
        Ok(record) => no_store(Json(AutomationGrantResponse::from(record)).into_response()),
        Err(failure) => no_store(ApiError::automation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn authorize_send(
    State(state): State<AutomationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(grant): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/automation-grants/{grant}/authorizations");
    let Some(grant_id) = grant_id(&grant) else {
        return invalid("automation.invalid_grant_id", correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid("automation.invalid_authorization_body", correlation_id);
    };
    let actor = match authenticate_signed_device_request(
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
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<AuthorizeAutomationSendBody>(&body) else {
        return invalid("automation.invalid_authorization_body", correlation_id);
    };
    let Some(request) = body.into_request(actor, grant_id) else {
        return invalid("automation.invalid_authorization_body", correlation_id);
    };
    match state.automation.authorize_send(request).await {
        Ok(outcome) => {
            no_store(Json(AutomationAuthorizationResponse::from(outcome)).into_response())
        }
        Err(failure) => no_store(ApiError::automation(failure, correlation_id).into_response()),
    }
}

async fn authenticate_write(
    state: &AutomationHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    if !origin_matches(headers, &state.frontend_origin) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "automation.invalid_origin",
                ErrorCategory::Authorization,
                "自动发言授权请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}
