use std::sync::Arc;

use agent_room_application::{
    handoffs::{
        AuthorizeHandoff, HandoffAccessUseCases, HandoffAuthorizationDecision,
        ResolveHandoffInstance, ResolvedHandoffInstance,
    },
    ports::SecretFactory,
};
use agent_room_domain::ids::{AgentId, AgentInstanceId, PrincipalId};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::no_store, devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const MAX_HANDOFF_AUTHORIZATION_BODY_BYTES: usize = 8 * 1_024;

#[derive(Clone)]
pub(crate) struct HandoffHttpState {
    handoffs: Arc<dyn HandoffAccessUseCases>,
    devices: Arc<dyn agent_room_application::devices::DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
}

pub(crate) struct HandoffHttpDependencies {
    pub(crate) handoffs: Arc<dyn HandoffAccessUseCases>,
    pub(crate) devices: Arc<dyn agent_room_application::devices::DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl HandoffHttpState {
    pub(crate) fn new(dependencies: HandoffHttpDependencies) -> Self {
        Self {
            handoffs: dependencies.handoffs,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
        }
    }
}

pub(crate) fn router(state: HandoffHttpState) -> Router {
    Router::new()
        .route("/handoffs/authorization", post(authorize_handoff))
        .route(
            "/agent-instances/{instance_id}/handoff-address",
            get(resolve_handoff_instance),
        )
        .layer(DefaultBodyLimit::max(MAX_HANDOFF_AUTHORIZATION_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffAuthorizationBody {
    #[serde(rename = "principalId")]
    principal: String,
    #[serde(rename = "requesterAgentId")]
    requester_agent: String,
    #[serde(rename = "requesterInstanceId")]
    requester_instance: String,
    #[serde(rename = "targetAgentId")]
    target_agent: String,
    #[serde(rename = "targetInstanceId")]
    target_instance: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffAuthorizationResponse {
    decision: &'static str,
}

#[derive(Debug, Serialize)]
struct HandoffInstanceResponse {
    #[serde(rename = "agentId")]
    agent: String,
    #[serde(rename = "agentInstanceId")]
    instance: String,
    #[serde(rename = "matrixUserId")]
    matrix_user: String,
    #[serde(rename = "matrixDeviceId")]
    matrix_device: String,
}

async fn authorize_handoff(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    const REQUEST_TARGET: &str = "/handoffs/authorization";
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(invalid_authorization_body(correlation_id).into_response());
    };
    let actor = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "POST",
        REQUEST_TARGET,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<HandoffAuthorizationBody>(&body) else {
        return no_store(invalid_authorization_body(correlation_id).into_response());
    };
    let Ok(request) = authorization_request(actor, &body) else {
        return no_store(invalid_authorization_body(correlation_id).into_response());
    };
    match state.handoffs.authorize(request).await {
        Ok(decision) => {
            no_store(Json(HandoffAuthorizationResponse::from(decision)).into_response())
        }
        Err(failure) => no_store(ApiError::handoff_access(failure, correlation_id).into_response()),
    }
}

async fn resolve_handoff_instance(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(instance_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agent-instances/{instance_id}/handoff-address");
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(invalid_instance_id(correlation_id).into_response());
    };
    if !body.is_empty() {
        return no_store(invalid_instance_body(correlation_id).into_response());
    }
    let actor = match authenticate_signed_device_request(
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
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .handoffs
        .resolve_instance(ResolveHandoffInstance { actor, instance_id })
        .await
    {
        Ok(instance) => no_store(Json(HandoffInstanceResponse::from(instance)).into_response()),
        Err(failure) => no_store(ApiError::handoff_access(failure, correlation_id).into_response()),
    }
}

fn authorization_request(
    actor: agent_room_application::devices::AuthenticatedDevice,
    body: &HandoffAuthorizationBody,
) -> Result<AuthorizeHandoff, ()> {
    Ok(AuthorizeHandoff {
        actor,
        principal_id: parse_uuid_v7(&body.principal).map(PrincipalId::from_uuid)?,
        requester_agent_id: parse_uuid_v7(&body.requester_agent).map(AgentId::from_uuid)?,
        requester_instance_id: parse_uuid_v7(&body.requester_instance)
            .map(AgentInstanceId::from_uuid)?,
        target_agent_id: parse_uuid_v7(&body.target_agent).map(AgentId::from_uuid)?,
        target_instance_id: parse_uuid_v7(&body.target_instance).map(AgentInstanceId::from_uuid)?,
    })
}

fn invalid_authorization_body(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_authorization_body", correlation_id)
}

fn invalid_instance_id(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_instance_id", correlation_id)
}

fn invalid_instance_body(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_instance_body", correlation_id)
}

impl From<HandoffAuthorizationDecision> for HandoffAuthorizationResponse {
    fn from(value: HandoffAuthorizationDecision) -> Self {
        Self {
            decision: match value {
                HandoffAuthorizationDecision::Allowed => "allowed",
                HandoffAuthorizationDecision::Denied => "denied",
            },
        }
    }
}

impl From<ResolvedHandoffInstance> for HandoffInstanceResponse {
    fn from(value: ResolvedHandoffInstance) -> Self {
        Self {
            agent: value.agent_id.to_string(),
            instance: value.instance_id.to_string(),
            matrix_user: value.matrix_user_id,
            matrix_device: value.matrix_device_id,
        }
    }
}
