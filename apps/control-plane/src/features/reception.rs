use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
    },
};
use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    devices::DeviceAuthorizationUseCases,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::SecretFactory,
    reception::{ReceptionRepository, ReceptionRequest},
};
use agent_room_domain::ids::{AgentId, DeviceId, RoomCatalogId};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct ReceptionHttpState {
    pub(crate) repository: Arc<dyn ReceptionRepository>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
    pub(crate) trusted_origins: TrustedOrigins,
}
pub(crate) fn router(state: ReceptionHttpState) -> Router {
    Router::new()
        .route("/receptions", get(list))
        .route("/receptions/control", post(control))
        .route("/receptions/transfer", post(transfer))
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(state)
}
async fn list(
    State(state): State<ReceptionHttpState>,
    Extension(correlation): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state.repository.list(actor.principal_id).await {
        Ok(records) => no_store(
            Json(serde_json::json!({ "receptions": records, "limited": records.len() == 128 }))
                .into_response(),
        ),
        Err(error) => failure(&error, correlation),
    }
}
async fn control(
    State(state): State<ReceptionHttpState>,
    Extension(correlation): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(text) = std::str::from_utf8(&body) else {
        return invalid(correlation);
    };
    let actor = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "POST",
        "/receptions/control",
        text,
        correlation,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(request) = serde_json::from_slice::<ReceptionRequest>(&body) else {
        return invalid(correlation);
    };
    if !request.valid() {
        return invalid(correlation);
    }
    match state
        .repository
        .execute(actor.account.principal.id(), actor.device_id, &request)
        .await
    {
        Ok(record) => no_store(Json(record).into_response()),
        Err(error) => failure(&error, correlation),
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransferRequest {
    agent_id: Uuid,
    catalog_id: Uuid,
    #[serde(rename = "nextDeviceId")]
    next_device: Option<Uuid>,
}
async fn transfer(
    State(state): State<ReceptionHttpState>,
    Extension(correlation): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "reception.invalid_origin",
                ErrorCategory::Authorization,
                "请求来源无效。",
                correlation,
            )
            .into_response(),
        );
    }
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<TransferRequest>(&body) else {
        return invalid(correlation);
    };
    match state
        .repository
        .drain(
            actor.principal_id,
            AgentId::from_uuid(body.agent_id),
            RoomCatalogId::from_uuid(body.catalog_id),
            body.next_device.map(DeviceId::from_uuid),
        )
        .await
    {
        Ok(record) => no_store(Json(record).into_response()),
        Err(error) => failure(&error, correlation),
    }
}
fn invalid(correlation: CorrelationId) -> Response {
    no_store(ApiError::invalid_request("reception.invalid_request", correlation).into_response())
}
fn failure(error: &RepositoryError, correlation: CorrelationId) -> Response {
    let (status, code, category) = match error.kind() {
        RepositoryErrorKind::Conflict => (
            StatusCode::CONFLICT,
            "reception.execution_conflict",
            ErrorCategory::Conflict,
        ),
        RepositoryErrorKind::Forbidden => (
            StatusCode::FORBIDDEN,
            "reception.forbidden",
            ErrorCategory::Authorization,
        ),
        RepositoryErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            "reception.not_found",
            ErrorCategory::Validation,
        ),
        RepositoryErrorKind::Constraint => (
            StatusCode::BAD_REQUEST,
            "reception.invalid_request",
            ErrorCategory::Validation,
        ),
        RepositoryErrorKind::Unavailable | RepositoryErrorKind::CorruptData => (
            StatusCode::SERVICE_UNAVAILABLE,
            "reception.unavailable",
            ErrorCategory::DependencyUnavailable,
        ),
    };
    no_store(
        ApiError::new(
            status,
            code,
            category,
            "接待状态未能更新，请刷新后重试。",
            correlation,
        )
        .into_response(),
    )
}
