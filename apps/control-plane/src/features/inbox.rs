use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store},
};
use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::PersonalInboxRepository,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct InboxHttpState {
    pub(crate) repository: Arc<dyn PersonalInboxRepository>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
}

pub(crate) fn router(state: InboxHttpState) -> Router {
    Router::new().route("/inbox", get(index)).with_state(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomResponse {
    catalog_id: String,
    room_id: String,
    name: String,
    direct: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffResponse {
    handoff_id: String,
    room_id: String,
    message_id: String,
    agent_name: String,
    status: String,
    created_at_unix_ms: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InboxResponse {
    account_id: String,
    rooms: Vec<RoomResponse>,
    handoffs: Vec<HandoffResponse>,
    limited: bool,
}

async fn index(
    State(state): State<InboxHttpState>,
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
    match state.repository.index(actor.principal_id).await {
        Ok(index) => no_store(
            Json(InboxResponse {
                account_id: actor.matrix_user_id,
                rooms: index
                    .rooms
                    .into_iter()
                    .map(|room| RoomResponse {
                        catalog_id: room.catalog_id,
                        room_id: room.room_id,
                        name: room.name,
                        direct: room.direct,
                    })
                    .collect(),
                handoffs: index
                    .handoffs
                    .into_iter()
                    .map(|handoff| HandoffResponse {
                        handoff_id: handoff.handoff_id,
                        room_id: handoff.room_id,
                        message_id: handoff.message_id,
                        agent_name: handoff.agent_name,
                        status: handoff.status,
                        created_at_unix_ms: handoff.created_at_unix_ms,
                    })
                    .collect(),
                limited: index.limited,
            })
            .into_response(),
        ),
        Err(error) => {
            tracing::warn!(operation = "inbox.index", kind = ?error.kind(), "Inbox metadata unavailable");
            no_store(
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "inbox.unavailable",
                    ErrorCategory::DependencyUnavailable,
                    "收件箱暂时不可用。",
                    correlation_id,
                )
                .into_response(),
            )
        }
    }
}
