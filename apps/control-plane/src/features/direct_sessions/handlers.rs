use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement},
    direct_sessions::{
        DirectSessionResult, InspectDirectSession, ListDirectSessions, SetDirectAgentBlock,
    },
};
use axum::{
    Json,
    extract::{Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;

use super::{
    DirectSessionHttpState,
    models::{
        DirectContactResponse, DirectSessionListResponse, DirectSessionResponse,
        OpenDirectSessionBody, SetDirectBlockBody, agent_id, catalog_id,
    },
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store, origin_matches},
};

pub(super) async fn open(
    State(state): State<DirectSessionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<OpenDirectSessionBody>, JsonRejection>,
) -> Response {
    let actor = match authenticate_write(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(Json(body)) = body else {
        return invalid("direct_session.invalid_open_body", correlation_id);
    };
    let Some(request) = body.into_request(actor) else {
        return invalid("direct_session.invalid_target_agent", correlation_id);
    };
    respond_session(state.sessions.open(request).await, correlation_id)
}

pub(super) async fn inspect(
    State(state): State<DirectSessionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid("direct_session.invalid_catalog_id", correlation_id);
    };
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
    respond_session(
        state
            .sessions
            .inspect(InspectDirectSession { actor, catalog_id })
            .await,
        correlation_id,
    )
}

pub(super) async fn list(
    State(state): State<DirectSessionHttpState>,
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
    match state.sessions.list(ListDirectSessions { actor }).await {
        Ok(sessions) => no_store(
            (
                StatusCode::OK,
                Json(DirectSessionListResponse::from(sessions)),
            )
                .into_response(),
        ),
        Err(failure) => no_store(ApiError::direct_session(failure, correlation_id).into_response()),
    }
}

pub(super) async fn set_block(
    State(state): State<DirectSessionHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(agent): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<SetDirectBlockBody>, JsonRejection>,
) -> Response {
    let Some(target_agent_id) = agent_id(&agent) else {
        return invalid("direct_session.invalid_target_agent", correlation_id);
    };
    let actor = match authenticate_write(&state, &headers, &jar, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(Json(body)) = body else {
        return invalid("direct_session.invalid_block_body", correlation_id);
    };
    match state
        .sessions
        .set_block(SetDirectAgentBlock {
            actor,
            target_agent_id,
            blocked: body.blocked,
        })
        .await
    {
        Ok(contact) => {
            no_store((StatusCode::OK, Json(DirectContactResponse::from(contact))).into_response())
        }
        Err(failure) => no_store(ApiError::direct_session(failure, correlation_id).into_response()),
    }
}

async fn authenticate_write(
    state: &DirectSessionHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    if !origin_matches(headers, &state.frontend_origin) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "direct_session.invalid_origin",
                agent_room_protocol_conformance::generated::ErrorCategory::Authorization,
                "直接会话写请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
}

fn respond_session(
    result: DirectSessionResult<agent_room_application::direct_sessions::DirectSessionView>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(session) => {
            no_store((StatusCode::OK, Json(DirectSessionResponse::from(session))).into_response())
        }
        Err(failure) => no_store(ApiError::direct_session(failure, correlation_id).into_response()),
    }
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}
