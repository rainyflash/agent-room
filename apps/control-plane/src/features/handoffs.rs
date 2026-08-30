use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    handoffs::{
        AuthorizeHandoff, ClaimNextTargetedHandoff, CreateTargetedHandoff,
        CreateTargetedHandoffOutcome, GetTargetedHandoff, HandoffAccessUseCases,
        HandoffAuthorizationDecision, HandoffTargetView, ListTargetedHandoffTargets,
        RecordTargetedHandoffReceiptCommand, ResolveHandoffInstance, ResolvedHandoffInstance,
        RevokeTargetedHandoff, TargetedHandoffUseCases,
    },
    ports::{SecretFactory, TargetedHandoffReceiptOutcome},
};
use agent_room_domain::{
    handoff::{
        HandoffFailureCode, HandoffPermission, HandoffPermissions, HandoffPurpose,
        HandoffSourceEventId, TargetedHandoff,
    },
    ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        content::sha256_hex,
        devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const MAX_HANDOFF_AUTHORIZATION_BODY_BYTES: usize = 8 * 1_024;

#[derive(Clone)]
pub(crate) struct HandoffHttpState {
    access: Arc<dyn HandoffAccessUseCases>,
    targeted: Arc<dyn TargetedHandoffUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    devices: Arc<dyn agent_room_application::devices::DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct HandoffHttpDependencies {
    pub(crate) handoffs: Arc<dyn HandoffAccessUseCases>,
    pub(crate) targeted: Arc<dyn TargetedHandoffUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn agent_room_application::devices::DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl HandoffHttpState {
    pub(crate) fn new(
        dependencies: HandoffHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            access: dependencies.handoffs,
            targeted: dependencies.targeted,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
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
        .route("/handoff-targets", get(list_handoff_targets))
        .route("/handoffs", post(create_handoff))
        .route(
            "/handoffs/{handoff_id}",
            get(get_handoff).delete(revoke_handoff),
        )
        .route(
            "/agent-instances/{instance_id}/handoffs/claim",
            post(claim_handoff),
        )
        .route(
            "/agent-instances/{instance_id}/handoffs/{handoff_id}/receipt",
            put(record_handoff_receipt),
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HandoffTargetQuery {
    room_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateHandoffBody {
    source_room_id: String,
    source_event_id: String,
    source_message_id: String,
    target_instance_id: String,
    content_id: String,
    permissions: Vec<String>,
    purpose: String,
    expires_at_unix_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HandoffReceiptBody {
    status: String,
    #[serde(default)]
    failure_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffTargetListResponse {
    targets: Vec<HandoffTargetResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffTargetResponse {
    agent_id: String,
    agent_display_name: String,
    agent_avatar_content_id: Option<String>,
    agent_instance_id: String,
    instance_status: &'static str,
    online: bool,
    adapter_type: String,
    capability_version: String,
    lease_expires_at_unix_ms: Option<i64>,
    last_seen_at_unix_ms: Option<i64>,
    device: HandoffTargetDeviceResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffTargetDeviceResponse {
    device_id: String,
    label: String,
    platform: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffResponse {
    handoff_id: String,
    principal_id: String,
    status: &'static str,
    source: HandoffSourceResponse,
    target: HandoffTargetReferenceResponse,
    content: HandoffContentResponse,
    permissions: Vec<&'static str>,
    purpose: &'static str,
    created_at_unix_ms: i64,
    queued_at_unix_ms: i64,
    delivered_at_unix_ms: Option<i64>,
    consumed_at_unix_ms: Option<i64>,
    resolved_at_unix_ms: Option<i64>,
    expires_at_unix_ms: i64,
    failure_code: Option<String>,
    version: u64,
}

#[derive(Debug, Serialize)]
struct HandoffSourceResponse {
    #[serde(rename = "matrixRoomId")]
    room: String,
    #[serde(rename = "matrixEventId")]
    event: String,
    #[serde(rename = "messageId")]
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffTargetReferenceResponse {
    agent_id: String,
    agent_instance_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffContentResponse {
    content_id: String,
    sha256: String,
    byte_length: u64,
    media_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateHandoffResponse {
    #[serde(flatten)]
    handoff: HandoffResponse,
    created: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaimHandoffResponse {
    handoff: Option<HandoffResponse>,
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
    match state.access.authorize(request).await {
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
        .access
        .resolve_instance(ResolveHandoffInstance { actor, instance_id })
        .await
    {
        Ok(instance) => no_store(Json(HandoffInstanceResponse::from(instance)).into_response()),
        Err(failure) => no_store(ApiError::handoff_access(failure, correlation_id).into_response()),
    }
}

async fn list_handoff_targets(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
    query: Result<Query<HandoffTargetQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(invalid_target_query(correlation_id).into_response());
    };
    let Ok(room_id) = MatrixRoomReference::new(query.room_id) else {
        return no_store(invalid_target_query(correlation_id).into_response());
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
    match state
        .targeted
        .list_targets(ListTargetedHandoffTargets { actor, room_id })
        .await
    {
        Ok(targets) => no_store(
            Json(HandoffTargetListResponse {
                targets: targets
                    .into_iter()
                    .map(HandoffTargetResponse::from)
                    .collect(),
            })
            .into_response(),
        ),
        Err(failure) => {
            no_store(ApiError::targeted_handoff(failure, correlation_id).into_response())
        }
    }
}

async fn create_handoff(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(invalid_origin(correlation_id).into_response());
    }
    let Some(handoff_id) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_uuid_v7(value).ok())
        .map(HandoffId::from_uuid)
    else {
        return no_store(invalid_create_body(correlation_id).into_response());
    };
    let Ok(body) = serde_json::from_slice::<CreateHandoffBody>(&body) else {
        return no_store(invalid_create_body(correlation_id).into_response());
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
    let Ok(request) = create_handoff_request(handoff_id, actor, body) else {
        return no_store(invalid_create_body(correlation_id).into_response());
    };
    match state.targeted.create(request).await {
        Ok(outcome) => create_handoff_response(outcome),
        Err(failure) => {
            no_store(ApiError::targeted_handoff(failure, correlation_id).into_response())
        }
    }
}

async fn get_handoff(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(handoff_id): Path<String>,
    jar: CookieJar,
) -> Response {
    let Ok(handoff_id) = parse_uuid_v7(&handoff_id).map(HandoffId::from_uuid) else {
        return no_store(invalid_handoff_id(correlation_id).into_response());
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
    match state
        .targeted
        .get(GetTargetedHandoff { actor, handoff_id })
        .await
    {
        Ok(handoff) => no_store(Json(HandoffResponse::from(handoff)).into_response()),
        Err(failure) => {
            no_store(ApiError::targeted_handoff(failure, correlation_id).into_response())
        }
    }
}

async fn revoke_handoff(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(handoff_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(invalid_origin(correlation_id).into_response());
    }
    let Ok(handoff_id) = parse_uuid_v7(&handoff_id).map(HandoffId::from_uuid) else {
        return no_store(invalid_handoff_id(correlation_id).into_response());
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
    match state
        .targeted
        .revoke(RevokeTargetedHandoff { actor, handoff_id })
        .await
    {
        Ok(handoff) => no_store(Json(HandoffResponse::from(handoff)).into_response()),
        Err(failure) => {
            no_store(ApiError::targeted_handoff(failure, correlation_id).into_response())
        }
    }
}

async fn claim_handoff(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(instance_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agent-instances/{instance_id}/handoffs/claim");
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
        "POST",
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
        .targeted
        .claim_next(ClaimNextTargetedHandoff {
            actor,
            target_instance_id: instance_id,
        })
        .await
    {
        Ok(handoff) => no_store(
            Json(ClaimHandoffResponse {
                handoff: handoff.map(HandoffResponse::from),
            })
            .into_response(),
        ),
        Err(failure) => {
            no_store(ApiError::targeted_handoff(failure, correlation_id).into_response())
        }
    }
}

async fn record_handoff_receipt(
    State(state): State<HandoffHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((instance_id, handoff_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agent-instances/{instance_id}/handoffs/{handoff_id}/receipt");
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(invalid_instance_id(correlation_id).into_response());
    };
    let Ok(handoff_id) = parse_uuid_v7(&handoff_id).map(HandoffId::from_uuid) else {
        return no_store(invalid_handoff_id(correlation_id).into_response());
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(invalid_receipt_body(correlation_id).into_response());
    };
    let actor = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "PUT",
        &request_target,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<HandoffReceiptBody>(&body) else {
        return no_store(invalid_receipt_body(correlation_id).into_response());
    };
    let Ok(outcome) = receipt_outcome(body) else {
        return no_store(invalid_receipt_body(correlation_id).into_response());
    };
    match state
        .targeted
        .record_receipt(RecordTargetedHandoffReceiptCommand {
            actor,
            target_instance_id: instance_id,
            handoff_id,
            outcome,
        })
        .await
    {
        Ok(handoff) => no_store(Json(HandoffResponse::from(handoff)).into_response()),
        Err(failure) => {
            no_store(ApiError::targeted_handoff(failure, correlation_id).into_response())
        }
    }
}

fn create_handoff_request(
    handoff_id: HandoffId,
    actor: agent_room_application::authentication::AuthenticatedPrincipal,
    body: CreateHandoffBody,
) -> Result<CreateTargetedHandoff, ()> {
    let permissions = body
        .permissions
        .iter()
        .map(|permission| HandoffPermission::try_from(permission.as_str()).map_err(|_| ()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CreateTargetedHandoff {
        handoff_id,
        actor,
        source_room_id: MatrixRoomReference::new(body.source_room_id).map_err(|_| ())?,
        source_event_id: HandoffSourceEventId::new(body.source_event_id).map_err(|_| ())?,
        source_message_id: parse_uuid_v7(&body.source_message_id).map(MessageId::from_uuid)?,
        target_instance_id: parse_uuid_v7(&body.target_instance_id)
            .map(AgentInstanceId::from_uuid)?,
        content_id: parse_uuid_v7(&body.content_id).map(ContentId::from_uuid)?,
        permissions: HandoffPermissions::new(permissions).map_err(|_| ())?,
        purpose: HandoffPurpose::try_from(body.purpose.as_str()).map_err(|_| ())?,
        expires_at: UtcMillis::new(body.expires_at_unix_ms).map_err(|_| ())?,
    })
}

fn receipt_outcome(body: HandoffReceiptBody) -> Result<TargetedHandoffReceiptOutcome, ()> {
    match (body.status.as_str(), body.failure_code) {
        ("consumed", None) => Ok(TargetedHandoffReceiptOutcome::Consumed),
        ("declined", Some(code)) => Ok(TargetedHandoffReceiptOutcome::Declined(
            HandoffFailureCode::new(code).map_err(|_| ())?,
        )),
        ("failed", Some(code)) => Ok(TargetedHandoffReceiptOutcome::Failed(
            HandoffFailureCode::new(code).map_err(|_| ())?,
        )),
        _ => Err(()),
    }
}

fn create_handoff_response(outcome: CreateTargetedHandoffOutcome) -> Response {
    let status = if outcome.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    no_store(
        (
            status,
            Json(CreateHandoffResponse {
                handoff: HandoffResponse::from(outcome.handoff),
                created: outcome.created,
            }),
        )
            .into_response(),
    )
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

fn invalid_target_query(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_target_query", correlation_id)
}

fn invalid_create_body(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_create_body", correlation_id)
}

fn invalid_handoff_id(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_id", correlation_id)
}

fn invalid_receipt_body(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("handoff.invalid_receipt_body", correlation_id)
}

fn invalid_origin(correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "handoff.invalid_origin",
        agent_room_protocol_conformance::generated::ErrorCategory::Authorization,
        "交接请求来源无效。",
        correlation_id,
    )
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

impl From<HandoffTargetView> for HandoffTargetResponse {
    fn from(value: HandoffTargetView) -> Self {
        Self {
            agent_id: value.record.agent_id.to_string(),
            agent_display_name: value.record.agent_display_name,
            agent_avatar_content_id: value
                .record
                .agent_avatar_content_id
                .map(|content_id| content_id.to_string()),
            agent_instance_id: value.record.instance_id.to_string(),
            instance_status: value.record.instance_status.as_str(),
            online: value.online,
            adapter_type: value.record.adapter_type,
            capability_version: value.record.capability_version,
            lease_expires_at_unix_ms: value.record.lease_expires_at.map(UtcMillis::value),
            last_seen_at_unix_ms: value.record.last_seen_at.map(UtcMillis::value),
            device: HandoffTargetDeviceResponse {
                device_id: value.record.device_id.to_string(),
                label: value.record.device_label,
                platform: value.record.device_platform.as_str(),
            },
        }
    }
}

impl From<TargetedHandoff> for HandoffResponse {
    fn from(value: TargetedHandoff) -> Self {
        let fields = value.fields();
        Self {
            handoff_id: fields.id.to_string(),
            principal_id: fields.principal_id.to_string(),
            status: value.status().as_str(),
            source: HandoffSourceResponse {
                room: fields.source_room_id.as_str().to_owned(),
                event: fields.source_event_id.as_str().to_owned(),
                message: fields.source_message_id.to_string(),
            },
            target: HandoffTargetReferenceResponse {
                agent_id: fields.target_agent_id.to_string(),
                agent_instance_id: fields.target_instance_id.to_string(),
            },
            content: HandoffContentResponse {
                content_id: fields.content.content_id().to_string(),
                sha256: sha256_hex(fields.content.digest()),
                byte_length: fields.content.byte_length().value(),
                media_type: fields.content.media_type().as_str().to_owned(),
            },
            permissions: fields
                .permissions
                .iter()
                .map(HandoffPermission::as_str)
                .collect(),
            purpose: fields.purpose.as_str(),
            created_at_unix_ms: fields.created_at.value(),
            queued_at_unix_ms: value.queued_at().value(),
            delivered_at_unix_ms: value.delivered_at().map(UtcMillis::value),
            consumed_at_unix_ms: value.consumed_at().map(UtcMillis::value),
            resolved_at_unix_ms: value.resolved_at().map(UtcMillis::value),
            expires_at_unix_ms: fields.expires_at.value(),
            failure_code: value.failure_code().map(|code| code.as_str().to_owned()),
            version: value.version(),
        }
    }
}
