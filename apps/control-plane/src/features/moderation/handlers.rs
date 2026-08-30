use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement},
    moderation::{
        InspectModerationCapabilities, ListModerationAudit, ListMyModerationCases,
        ListRoomModeration, ListRoomModerationCases, ReverseModerationAction,
    },
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json,
    extract::{Extension, Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;

use super::{
    ModerationHttpState,
    models::{
        ApplyActionBody, AuditQuery, ModerationActionListResponse, ModerationActionResponse,
        ModerationAuditListResponse, ModerationCapabilitiesResponse, ModerationCaseListResponse,
        ModerationCaseResponse, ReverseActionBody, SubmitReportBody, action_id, case_id,
        catalog_id,
    },
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store, origin_matches},
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

pub(super) async fn submit_report(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<SubmitReportBody>, JsonRejection>,
) -> Response {
    let actor = match authenticate_write(
        &state,
        &headers,
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(id) = idempotency_value(&headers).and_then(case_id) else {
        return invalid("moderation.invalid_idempotency_key", correlation_id);
    };
    let Ok(Json(body)) = body else {
        return invalid("moderation.invalid_report_body", correlation_id);
    };
    let Some(request) = body.into_request(actor, id) else {
        return invalid("moderation.invalid_report_body", correlation_id);
    };
    match state.moderation.submit_report(request).await {
        Ok(case) => no_store(
            (
                StatusCode::CREATED,
                Json(ModerationCaseResponse::from(case)),
            )
                .into_response(),
        ),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn list_cases(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
    let actor = match authenticate_read(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .moderation
        .list_my_cases(ListMyModerationCases { actor })
        .await
    {
        Ok(cases) => no_store(Json(ModerationCaseListResponse::from(cases)).into_response()),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn inspect_capabilities(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(room_catalog_id) = catalog_id(&catalog) else {
        return invalid("moderation.invalid_catalog_id", correlation_id);
    };
    let actor = match authenticate_read(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .moderation
        .inspect_capabilities(InspectModerationCapabilities {
            actor,
            room_catalog_id,
        })
        .await
    {
        Ok(capabilities) => {
            no_store(Json(ModerationCapabilitiesResponse::from(capabilities)).into_response())
        }
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn apply_action(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<ApplyActionBody>, JsonRejection>,
) -> Response {
    let Some(room_catalog_id) = catalog_id(&catalog) else {
        return invalid("moderation.invalid_catalog_id", correlation_id);
    };
    let actor = match authenticate_write(
        &state,
        &headers,
        &jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(id) = idempotency_value(&headers).and_then(action_id) else {
        return invalid("moderation.invalid_idempotency_key", correlation_id);
    };
    let Ok(Json(body)) = body else {
        return invalid("moderation.invalid_action_body", correlation_id);
    };
    let Some(request) = body.into_request(actor, id, room_catalog_id) else {
        return invalid("moderation.invalid_action_body", correlation_id);
    };
    match state.moderation.apply_action(request).await {
        Ok(action) => no_store(
            (
                StatusCode::CREATED,
                Json(ModerationActionResponse::from(action)),
            )
                .into_response(),
        ),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn reverse_action(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(action): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<ReverseActionBody>, JsonRejection>,
) -> Response {
    let Some(action_id) = action_id(&action) else {
        return invalid("moderation.invalid_action_id", correlation_id);
    };
    let actor = match authenticate_write(
        &state,
        &headers,
        &jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(Json(body)) = body else {
        return invalid("moderation.invalid_reverse_body", correlation_id);
    };
    match state
        .moderation
        .reverse_action(ReverseModerationAction {
            actor,
            action_id,
            impact_acknowledged: body.impact_acknowledged(),
        })
        .await
    {
        Ok(action) => no_store(Json(ModerationActionResponse::from(action)).into_response()),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn list_actions(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(room_catalog_id) = catalog_id(&catalog) else {
        return invalid("moderation.invalid_catalog_id", correlation_id);
    };
    let actor = match authenticate_read(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .moderation
        .list_room_actions(ListRoomModeration {
            actor,
            room_catalog_id,
        })
        .await
    {
        Ok(actions) => no_store(Json(ModerationActionListResponse::from(actions)).into_response()),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn list_room_cases(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(room_catalog_id) = catalog_id(&catalog) else {
        return invalid("moderation.invalid_catalog_id", correlation_id);
    };
    let actor = match authenticate_read(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .moderation
        .list_room_cases(ListRoomModerationCases {
            actor,
            room_catalog_id,
        })
        .await
    {
        Ok(cases) => no_store(Json(ModerationCaseListResponse::from(cases)).into_response()),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

pub(super) async fn list_audit(
    State(state): State<ModerationHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
    query: Result<Query<AuditQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let actor = match authenticate_read(&state, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(Query(query)) = query else {
        return invalid("moderation.invalid_audit_query", correlation_id);
    };
    let Ok(room_catalog_id) = query.room_catalog_id() else {
        return invalid("moderation.invalid_audit_query", correlation_id);
    };
    match state
        .moderation
        .list_audit(ListModerationAudit {
            actor,
            room_catalog_id,
            limit: query.limit(),
        })
        .await
    {
        Ok(events) => no_store(Json(ModerationAuditListResponse::from(events)).into_response()),
        Err(failure) => no_store(ApiError::moderation(failure, correlation_id).into_response()),
    }
}

async fn authenticate_read(
    state: &ModerationHttpState,
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

async fn authenticate_write(
    state: &ModerationHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    requirement: AuthenticationRequirement,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "moderation.invalid_origin",
                ErrorCategory::Authorization,
                "治理请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        requirement,
        correlation_id,
    )
    .await
}

fn idempotency_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}
