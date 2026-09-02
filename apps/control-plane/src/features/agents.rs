use std::sync::Arc;

use agent_room_application::{
    agent_instance_verification::{
        AgentInstanceVerificationUseCases, ResolveAgentInstanceVerification,
    },
    agents::{
        AgentManagementUseCases, ChangeAgentMembership, CreateAgent, EnsureDefaultAgent,
        EnsureDefaultAgentForDevice, ListAgents, RegisterAgentInstance, RegisteredAgentInstance,
        RotateAgentInstanceMatrixSession, RotatedAgentInstanceMatrixSession,
    },
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    devices::DeviceAuthorizationUseCases,
    ports::{AgentInstanceVerificationRecord, SecretFactory},
};
use agent_room_domain::{
    agents::{AdapterSubjectHash, AgentInstancePublicSigningKey, AgentRole, AgentVisibility},
    ids::{
        AgentCreationRequestId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId,
        ContentId, PrincipalId,
    },
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use axum_extra::extract::CookieJar;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const MAX_AGENT_BODY_BYTES: usize = 64 * 1_024;
const MAX_ENCODED_HASH_LENGTH: usize = 64;
const MAX_ENCODED_PUBLIC_KEY_LENGTH: usize = 64;
const DEVICE_DEFAULT_AGENT_TARGET: &str = "/onboarding/device/default-agent";
const TARGETED_HANDOFF_CAPABILITY: &str = "targeted_handoff_v1";

#[derive(Clone)]
pub(crate) struct AgentHttpState {
    agents: Arc<dyn AgentManagementUseCases>,
    verification: Arc<dyn AgentInstanceVerificationUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct AgentHttpDependencies {
    pub(crate) agents: Arc<dyn AgentManagementUseCases>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl AgentHttpState {
    pub(crate) fn new(
        dependencies: AgentHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            agents: dependencies.agents,
            verification: dependencies.verification,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
        }
    }
}

pub(crate) fn router(state: AgentHttpState) -> Router {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/onboarding/default-agent", put(ensure_default_agent))
        .route(
            DEVICE_DEFAULT_AGENT_TARGET,
            put(ensure_default_agent_for_device),
        )
        .route(
            "/agents/{agent_id}/members/{principal_id}",
            put(grant_membership).delete(revoke_membership),
        )
        .route("/agents/{agent_id}/instances", post(register_instance))
        .route(
            "/agent-instances/{instance_id}/matrix-session",
            post(rotate_instance_matrix_session),
        )
        .route(
            "/agent-instances/{instance_id}/verification",
            get(resolve_instance_verification),
        )
        .layer(DefaultBodyLimit::max(MAX_AGENT_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateAgentBody {
    slug: String,
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    avatar_content_id: Option<String>,
    visibility: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MembershipBody {
    role: String,
}

struct MembershipRouteTarget {
    agent_id: String,
    principal_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisterInstanceBody {
    adapter_type: String,
    #[serde(default)]
    external_subject_hash: Option<String>,
    capability_version: String,
    #[serde(default)]
    configuration: Map<String, Value>,
    public_signing_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentResponse {
    agent_id: String,
    matrix_user_id: String,
    slug: String,
    display_name: String,
    description: String,
    avatar_content_id: Option<String>,
    visibility: &'static str,
    registered_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentListResponse {
    agents: Vec<AgentResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstanceResponse {
    agent_id: String,
    display_name: String,
    avatar_content_id: Option<String>,
    adapter_binding_id: String,
    agent_instance_id: String,
    matrix_user_id: String,
    matrix_device_id: String,
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentInstanceVerificationResponse {
    agent_instance_id: String,
    agent_id: String,
    public_signing_key: String,
    registered_at_unix_ms: i64,
    invalidated_at_unix_ms: Option<i64>,
}

async fn create_agent(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<CreateAgentBody>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(invalid_origin(correlation_id).into_response());
    }
    let Ok(request_id) = creation_request_id(&headers) else {
        return no_store(invalid_idempotency_key(correlation_id).into_response());
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
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::invalid_request("agent.invalid_creation_body", correlation_id)
                .into_response(),
        );
    };
    let Ok(request) = create_agent_request(request_id, actor, body) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_creation_body", correlation_id)
                .into_response(),
        );
    };
    match state.agents.create_agent(request).await {
        Ok(agent) => {
            no_store((StatusCode::CREATED, Json(AgentResponse::from(agent))).into_response())
        }
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn list_agents(
    State(state): State<AgentHttpState>,
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
    match state.agents.list_agents(ListAgents { actor }).await {
        Ok(agents) => no_store(
            Json(AgentListResponse {
                agents: agents.into_iter().map(AgentResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn ensure_default_agent(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(invalid_origin(correlation_id).into_response());
    }
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
        .agents
        .ensure_default_agent(EnsureDefaultAgent { actor })
        .await
    {
        Ok(agent) => no_store(Json(AgentResponse::from(agent)).into_response()),
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn ensure_default_agent_for_device(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_default_agent_body", correlation_id)
                .into_response(),
        );
    };
    let actor = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "PUT",
        DEVICE_DEFAULT_AGENT_TARGET,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    if !body.is_empty() {
        return no_store(
            ApiError::invalid_request("agent.invalid_default_agent_body", correlation_id)
                .into_response(),
        );
    }
    match state
        .agents
        .ensure_default_agent_for_device(EnsureDefaultAgentForDevice { actor })
        .await
    {
        Ok(agent) => no_store(Json(AgentResponse::from(agent)).into_response()),
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn grant_membership(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((agent_id, principal_id)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<MembershipBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::invalid_request("agent.invalid_membership_body", correlation_id)
                .into_response(),
        );
    };
    let Ok(role) = AgentRole::try_from(body.role.as_str()) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_membership_role", correlation_id)
                .into_response(),
        );
    };
    change_membership(
        &state,
        correlation_id,
        &headers,
        &jar,
        MembershipRouteTarget {
            agent_id,
            principal_id,
        },
        Some(role),
    )
    .await
}

async fn revoke_membership(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((agent_id, principal_id)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    change_membership(
        &state,
        correlation_id,
        &headers,
        &jar,
        MembershipRouteTarget {
            agent_id,
            principal_id,
        },
        None,
    )
    .await
}

async fn change_membership(
    state: &AgentHttpState,
    correlation_id: CorrelationId,
    headers: &HeaderMap,
    jar: &CookieJar,
    target: MembershipRouteTarget,
    role: Option<AgentRole>,
) -> Response {
    if !origin_matches(headers, &state.trusted_origins) {
        return no_store(invalid_origin(correlation_id).into_response());
    }
    let Ok(agent_id) = parse_uuid_v7(&target.agent_id).map(AgentId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(principal_id) = parse_uuid_v7(&target.principal_id).map(PrincipalId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .agents
        .change_membership(ChangeAgentMembership {
            actor,
            agent_id,
            principal_id,
            role,
        })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn register_instance(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agents/{agent_id}/instances");
    let Ok(agent_id) = parse_uuid_v7(&agent_id).map(AgentId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(request_id) = instance_request_id(&headers) else {
        return no_store(invalid_idempotency_key(correlation_id).into_response());
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_instance_body", correlation_id)
                .into_response(),
        );
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
    let Ok(body) = serde_json::from_slice::<RegisterInstanceBody>(&body) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_instance_body", correlation_id)
                .into_response(),
        );
    };
    let Ok(request) = register_instance_request(request_id, actor, agent_id, body) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_instance_body", correlation_id)
                .into_response(),
        );
    };
    match state.agents.register_instance(request).await {
        Ok(instance) => no_store(Json(AgentInstanceResponse::from(instance)).into_response()),
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn rotate_instance_matrix_session(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(instance_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agent-instances/{instance_id}/matrix-session");
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return no_store(
            ApiError::invalid_request("agent.invalid_matrix_session_body", correlation_id)
                .into_response(),
        );
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
    if !body.is_empty() {
        return no_store(
            ApiError::invalid_request("agent.invalid_matrix_session_body", correlation_id)
                .into_response(),
        );
    }
    match state
        .agents
        .rotate_instance_matrix_session(RotateAgentInstanceMatrixSession { actor, instance_id })
        .await
    {
        Ok(session) => no_store(Json(AgentInstanceResponse::from(session)).into_response()),
        Err(failure) => no_store(ApiError::agent(failure, correlation_id).into_response()),
    }
}

async fn resolve_instance_verification(
    State(state): State<AgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(instance_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agent-instances/{instance_id}/verification");
    let Ok(instance_id) = parse_uuid_v7(&instance_id).map(AgentInstanceId::from_uuid) else {
        return no_store(invalid_resource_id(correlation_id).into_response());
    };
    if !body.is_empty() {
        return no_store(
            ApiError::invalid_request("agent.invalid_verification_body", correlation_id)
                .into_response(),
        );
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
        .verification
        .resolve(ResolveAgentInstanceVerification { actor, instance_id })
        .await
    {
        Ok(record) => {
            no_store(Json(AgentInstanceVerificationResponse::from(record)).into_response())
        }
        Err(failure) => {
            no_store(ApiError::agent_instance_verification(failure, correlation_id).into_response())
        }
    }
}

fn create_agent_request(
    request_id: AgentCreationRequestId,
    actor: agent_room_application::authentication::AuthenticatedPrincipal,
    body: CreateAgentBody,
) -> Result<CreateAgent, ()> {
    let avatar_content_id = body
        .avatar_content_id
        .map(|value| parse_uuid_v7(&value).map(ContentId::from_uuid))
        .transpose()?;
    let visibility = AgentVisibility::try_from(body.visibility.as_str()).map_err(|_| ())?;
    Ok(CreateAgent {
        request_id,
        actor,
        slug: body.slug,
        display_name: body.display_name,
        description: body.description,
        avatar_content_id,
        visibility,
    })
}

fn register_instance_request(
    request_id: AgentInstanceRegistrationRequestId,
    actor: agent_room_application::devices::AuthenticatedDevice,
    agent_id: AgentId,
    body: RegisterInstanceBody,
) -> Result<RegisterAgentInstance, ()> {
    let configuration = validate_adapter_configuration(body.configuration)?;
    let external_subject_hash = body
        .external_subject_hash
        .map(|value| {
            decode_bounded(&value, MAX_ENCODED_HASH_LENGTH)
                .and_then(|bytes| AdapterSubjectHash::new(bytes).map_err(|_| ()))
        })
        .transpose()?;
    let public_signing_key =
        decode_bounded(&body.public_signing_key, MAX_ENCODED_PUBLIC_KEY_LENGTH)
            .and_then(|bytes| AgentInstancePublicSigningKey::new(bytes).map_err(|_| ()))?;
    Ok(RegisterAgentInstance {
        request_id,
        actor,
        agent_id,
        adapter_type: body.adapter_type,
        external_subject_hash,
        capability_version: body.capability_version,
        configuration,
        public_signing_key,
    })
}

fn validate_adapter_configuration(
    configuration: Map<String, Value>,
) -> Result<Map<String, Value>, ()> {
    if configuration.is_empty() {
        return Ok(configuration);
    }
    let valid = configuration.len() == 1
        && matches!(
            configuration.get("capabilities"),
            Some(Value::Array(capabilities))
                if capabilities.as_slice()
                    == [Value::String(TARGETED_HANDOFF_CAPABILITY.to_owned())]
        );
    valid.then_some(configuration).ok_or(())
}

fn creation_request_id(headers: &HeaderMap) -> Result<AgentCreationRequestId, ()> {
    idempotency_uuid(headers).map(AgentCreationRequestId::from_uuid)
}

fn instance_request_id(headers: &HeaderMap) -> Result<AgentInstanceRegistrationRequestId, ()> {
    idempotency_uuid(headers).map(AgentInstanceRegistrationRequestId::from_uuid)
}

fn idempotency_uuid(headers: &HeaderMap) -> Result<Uuid, ()> {
    let value = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(())?;
    parse_uuid_v7(value)
}

fn decode_bounded(value: &str, maximum_encoded_length: usize) -> Result<Vec<u8>, ()> {
    if value.is_empty() || value.len() > maximum_encoded_length {
        return Err(());
    }
    URL_SAFE_NO_PAD.decode(value).map_err(|_| ())
}

fn invalid_origin(correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "agent.invalid_origin",
        ErrorCategory::Authorization,
        "Agent 管理请求来源无效。",
        correlation_id,
    )
}

fn invalid_idempotency_key(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("agent.invalid_idempotency_key", correlation_id)
}

fn invalid_resource_id(correlation_id: CorrelationId) -> ApiError {
    ApiError::invalid_request("agent.invalid_resource_id", correlation_id)
}

impl From<agent_room_application::ports::RegisteredAgent> for AgentResponse {
    fn from(value: agent_room_application::ports::RegisteredAgent) -> Self {
        Self {
            agent_id: value.agent.id().to_string(),
            matrix_user_id: value.matrix_user_id,
            slug: value.slug,
            display_name: value.display_name,
            description: value.description,
            avatar_content_id: value.avatar_content_id.map(|id| id.to_string()),
            visibility: value.visibility.as_str(),
            registered_at_unix_ms: value.registered_at.value(),
        }
    }
}

impl From<RegisteredAgentInstance> for AgentInstanceResponse {
    fn from(value: RegisteredAgentInstance) -> Self {
        Self {
            agent_id: value.registration.instance.agent_id().to_string(),
            display_name: value.agent.display_name,
            avatar_content_id: value.agent.avatar_content_id.map(|id| id.to_string()),
            adapter_binding_id: value.registration.binding.id().to_string(),
            agent_instance_id: value.registration.instance.id().to_string(),
            matrix_user_id: value
                .matrix_session
                .metadata()
                .user_id()
                .as_str()
                .to_owned(),
            matrix_device_id: value
                .matrix_session
                .metadata()
                .device_id()
                .as_str()
                .to_owned(),
            access_token: value.matrix_session.access_token().expose().to_owned(),
            refresh_token: value
                .matrix_session
                .refresh_token()
                .map(|token| token.expose().to_owned()),
        }
    }
}

impl From<RotatedAgentInstanceMatrixSession> for AgentInstanceResponse {
    fn from(value: RotatedAgentInstanceMatrixSession) -> Self {
        Self {
            agent_id: value.instance.instance.agent_id().to_string(),
            display_name: value.instance.agent_display_name,
            avatar_content_id: value
                .instance
                .agent_avatar_content_id
                .map(|id| id.to_string()),
            adapter_binding_id: value.instance.instance.adapter_binding_id().to_string(),
            agent_instance_id: value.instance.instance.id().to_string(),
            matrix_user_id: value
                .matrix_session
                .metadata()
                .user_id()
                .as_str()
                .to_owned(),
            matrix_device_id: value
                .matrix_session
                .metadata()
                .device_id()
                .as_str()
                .to_owned(),
            access_token: value.matrix_session.access_token().expose().to_owned(),
            refresh_token: value
                .matrix_session
                .refresh_token()
                .map(|token| token.expose().to_owned()),
        }
    }
}

impl From<AgentInstanceVerificationRecord> for AgentInstanceVerificationResponse {
    fn from(value: AgentInstanceVerificationRecord) -> Self {
        Self {
            agent_instance_id: value.instance_id.to_string(),
            agent_id: value.agent_id.to_string(),
            public_signing_key: URL_SAFE_NO_PAD.encode(value.public_signing_key.as_bytes()),
            registered_at_unix_ms: value.registered_at.value(),
            invalidated_at_unix_ms: value.invalidated_at.map(UtcMillis::value),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use agent_room_application::{
        agent_instance_verification::{
            AgentInstanceVerificationResult, AgentInstanceVerificationUseCases,
            ResolveAgentInstanceVerification,
        },
        agents::{
            AgentManagementResult, AgentManagementUseCases, ChangeAgentMembership, CreateAgent,
            EnsureDefaultAgent, EnsureDefaultAgentForDevice, ListAgents, RegisterAgentInstance,
            RegisteredAgentInstance, RotateAgentInstanceMatrixSession,
            RotatedAgentInstanceMatrixSession,
        },
        authentication::{
            AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
            AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
        },
        devices::{
            AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
            DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
            RevokedDevice,
        },
        ports::{
            AgentInstanceManagementRecord, AgentInstanceVerificationRecord, MatrixDeviceId,
            MatrixSession, MatrixSessionMetadata, MatrixUserId, PortFuture, PrincipalAccount,
            RegisteredAgent, SecretFactory, SecretValue, StoredAgentInstanceRegistration,
        },
    };
    use agent_room_domain::{
        agents::{
            AdapterBinding, Agent, AgentInstance, AgentInstancePublicSigningKey,
            AgentMatrixDeviceId, AgentRole, AgentVisibility,
        },
        devices::{Device, DevicePlatform, DeviceTrustState},
        identity::Principal,
        ids::{AdapterBindingId, AgentId, AgentInstanceId, DeviceId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use url::Url;
    use uuid::Uuid;

    use super::{AgentHttpDependencies, AgentHttpState, DEVICE_DEFAULT_AGENT_TARGET, router};

    const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
    const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
    const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const REQUEST_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
    const BINDING_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const INSTANCE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
    const TARGET_PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
    const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
    const SESSION_COOKIE: &str = "__Host-agent-room-session=session-secret";

    #[derive(Default)]
    struct FakeAgents {
        creation: Mutex<Option<CreateAgent>>,
        default_agent_ensures: AtomicUsize,
        device_default_agent_ensures: AtomicUsize,
        registration: Mutex<Option<RegisterAgentInstance>>,
        rotation: Mutex<Option<RotateAgentInstanceMatrixSession>>,
        membership_changes: Mutex<Vec<ChangeAgentMembership>>,
    }

    impl AgentManagementUseCases for FakeAgents {
        fn list_agents(
            &self,
            _request: ListAgents,
        ) -> PortFuture<'_, AgentManagementResult<Vec<RegisteredAgent>>> {
            Box::pin(async { Ok(vec![registered_agent()]) })
        }

        fn ensure_default_agent(
            &self,
            _request: EnsureDefaultAgent,
        ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
            self.default_agent_ensures.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(registered_agent()) })
        }

        fn ensure_default_agent_for_device(
            &self,
            request: EnsureDefaultAgentForDevice,
        ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
            assert_eq!(request.actor.device_id, device_id());
            self.device_default_agent_ensures
                .fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(registered_agent()) })
        }

        fn create_agent(
            &self,
            request: CreateAgent,
        ) -> PortFuture<'_, AgentManagementResult<RegisteredAgent>> {
            *self.creation.lock().expect("Agent 创建记录锁可用") = Some(request);
            Box::pin(async { Ok(registered_agent()) })
        }

        fn register_instance(
            &self,
            request: RegisterAgentInstance,
        ) -> PortFuture<'_, AgentManagementResult<RegisteredAgentInstance>> {
            *self.registration.lock().expect("Agent 实例记录锁可用") = Some(request);
            Box::pin(async { Ok(registered_instance()) })
        }

        fn rotate_instance_matrix_session(
            &self,
            request: RotateAgentInstanceMatrixSession,
        ) -> PortFuture<'_, AgentManagementResult<RotatedAgentInstanceMatrixSession>> {
            *self.rotation.lock().expect("Matrix 会话轮换记录锁可用") = Some(request);
            Box::pin(async { Ok(rotated_instance()) })
        }

        fn change_membership(
            &self,
            request: ChangeAgentMembership,
        ) -> PortFuture<'_, AgentManagementResult<()>> {
            self.membership_changes
                .lock()
                .expect("Agent 成员变更记录锁可用")
                .push(request);
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Default)]
    struct FakeVerification {
        resolutions: Mutex<Vec<ResolveAgentInstanceVerification>>,
    }

    impl AgentInstanceVerificationUseCases for FakeVerification {
        fn resolve(
            &self,
            request: ResolveAgentInstanceVerification,
        ) -> PortFuture<'_, AgentInstanceVerificationResult<AgentInstanceVerificationRecord>>
        {
            self.resolutions
                .lock()
                .expect("实例验签查询记录锁可用")
                .push(request);
            Box::pin(async { Ok(verification_record()) })
        }
    }

    #[derive(Default)]
    struct FakeAuthentication {
        requirements: Mutex<Vec<AuthenticationRequirement>>,
    }

    impl AuthenticationUseCases for FakeAuthentication {
        fn begin_login(
            &self,
            _request: BeginLogin,
        ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
            Box::pin(async { unreachable!("Agent 路由不会开始浏览器登录") })
        }

        fn complete_login<'a>(
            &'a self,
            _request: CompleteLogin<'a>,
        ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
            Box::pin(async { unreachable!("Agent 路由不会完成浏览器登录") })
        }

        fn authenticate<'a>(
            &'a self,
            session_secret: &'a SecretValue,
            requirement: AuthenticationRequirement,
        ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
            assert_eq!(session_secret.expose(), "session-secret");
            self.requirements
                .lock()
                .expect("认证要求记录锁可用")
                .push(requirement);
            Box::pin(async { Ok(authenticated_principal()) })
        }

        fn logout<'a>(
            &'a self,
            _session_secret: &'a SecretValue,
        ) -> PortFuture<'a, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("Agent 路由不会退出浏览器登录") })
        }

        fn suspend_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("Agent 路由不会暂停主体") })
        }
    }

    #[derive(Default)]
    struct FakeDevices {
        authentications: AtomicUsize,
        expected_request: Mutex<Option<ExpectedDeviceRequest>>,
    }

    struct ExpectedDeviceRequest {
        method: String,
        target: String,
        body: String,
    }

    impl FakeDevices {
        fn expect_request(&self, method: &str, target: String, body: String) {
            *self
                .expected_request
                .lock()
                .expect("设备请求预期记录锁可用") = Some(ExpectedDeviceRequest {
                method: method.to_owned(),
                target,
                body,
            });
        }
    }

    impl DeviceAuthorizationUseCases for FakeDevices {
        fn register_device(
            &self,
            _request: RegisterDevice,
        ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
            Box::pin(async { unreachable!("Agent 路由不会注册用户设备") })
        }

        fn authenticate_device<'a>(
            &'a self,
            request: AuthenticateDeviceRequest<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
            self.authentications.fetch_add(1, Ordering::SeqCst);
            let expected = self
                .expected_request
                .lock()
                .expect("设备请求预期记录锁可用")
                .take()
                .expect("测试必须登记完整设备请求预期");
            assert_eq!(request.access_token.expose(), "device-access-token");
            assert_eq!(request.proof.device_id(), device_id());
            assert_eq!(request.proof.issued_at(), time(1_700_000_000_000));
            assert_eq!(request.proof.nonce().expose(), "nonce-0123456789abcdef");
            assert_eq!(request.proof.method(), expected.method);
            assert_eq!(request.proof.request_target(), expected.target);
            assert_eq!(
                request.proof.body_digest(),
                &SecureSecretFactory.digest(&expected.body)
            );
            Box::pin(async { Ok(authenticated_device()) })
        }

        fn refresh_device_session<'a>(
            &'a self,
            _request: RefreshDeviceSession<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
            Box::pin(async { unreachable!("Agent 路由不会刷新用户设备会话") })
        }

        fn list_devices(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
            Box::pin(async { unreachable!("Agent 路由不会列出用户设备") })
        }

        fn revoke_device(
            &self,
            _principal_id: PrincipalId,
            _device_id: DeviceId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
            Box::pin(async { unreachable!("Agent 路由不会撤销用户设备") })
        }
    }

    fn test_router(
        agents: Arc<FakeAgents>,
        authentication: Arc<FakeAuthentication>,
        devices: Arc<FakeDevices>,
    ) -> axum::Router {
        test_router_with_verification(
            agents,
            authentication,
            devices,
            Arc::new(FakeVerification::default()),
        )
    }

    fn test_router_with_verification(
        agents: Arc<FakeAgents>,
        authentication: Arc<FakeAuthentication>,
        devices: Arc<FakeDevices>,
        verification: Arc<FakeVerification>,
    ) -> axum::Router {
        let state = AgentHttpState::new(
            AgentHttpDependencies {
                agents,
                verification,
                authentication,
                devices,
                secrets: Arc::new(SecureSecretFactory),
            },
            &Url::parse(FRONTEND_ORIGIN).expect("前端 Origin 有效"),
            &Url::parse("http://tauri.localhost").expect("桌面 Origin 有效"),
        );
        router(state).layer(middleware::from_fn(crate::correlation::attach))
    }

    #[tokio::test]
    async fn 创建_agent_要求同源会话和_uuidv7_幂等键() {
        let agents = Arc::new(FakeAgents::default());
        let authentication = Arc::new(FakeAuthentication::default());
        let app = test_router(
            agents.clone(),
            authentication.clone(),
            Arc::new(FakeDevices::default()),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header(header::ORIGIN, FRONTEND_ORIGIN)
                    .header(header::COOKIE, SESSION_COOKIE)
                    .header("idempotency-key", REQUEST_UUID)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "slug": "codex-builder",
                            "displayName": "Codex Builder",
                            "description": "构建 Agent Room",
                            "visibility": "private"
                        })
                        .to_string(),
                    ))
                    .expect("创建 Agent 请求有效"),
            )
            .await
            .expect("创建 Agent 路由可调用");

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let payload = response_json(response).await;
        assert_eq!(payload["agentId"], AGENT_UUID);
        assert_eq!(payload["visibility"], "private");
        assert!(!payload.to_string().contains("application-service-token"));

        let creation = agents
            .creation
            .lock()
            .expect("Agent 创建记录锁可用")
            .clone()
            .expect("Agent 创建用例已调用");
        assert_eq!(creation.request_id.to_string(), REQUEST_UUID);
        assert_eq!(creation.actor.principal_id, principal_id());
        assert_eq!(creation.slug, "codex-builder");
        assert_eq!(creation.visibility, AgentVisibility::Private);
        assert_eq!(
            *authentication
                .requirements
                .lock()
                .expect("认证要求记录锁可用"),
            vec![AuthenticationRequirement::ActiveSession]
        );
    }

    #[tokio::test]
    async fn 首次引导可列出并幂等确保默认_agent() {
        let agents = Arc::new(FakeAgents::default());
        let app = test_router(
            agents.clone(),
            Arc::new(FakeAuthentication::default()),
            Arc::new(FakeDevices::default()),
        );

        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/agents")
                    .header(header::COOKIE, SESSION_COOKIE)
                    .body(Body::empty())
                    .expect("Agent 列表请求有效"),
            )
            .await
            .expect("Agent 列表路由可调用");
        assert_eq!(list_response.status(), StatusCode::OK);
        let payload = response_json(list_response).await;
        assert_eq!(payload["agents"][0]["agentId"], AGENT_UUID);

        let ensure_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/onboarding/default-agent")
                    .header(header::ORIGIN, FRONTEND_ORIGIN)
                    .header(header::COOKIE, SESSION_COOKIE)
                    .body(Body::empty())
                    .expect("默认 Agent 请求有效"),
            )
            .await
            .expect("默认 Agent 路由可调用");
        assert_eq!(ensure_response.status(), StatusCode::OK);
        assert_eq!(response_json(ensure_response).await["agentId"], AGENT_UUID);
        assert_eq!(agents.default_agent_ensures.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn 桌面首次引导使用设备持有证明而不依赖_web_cookie_或_origin() {
        let agents = Arc::new(FakeAgents::default());
        let authentication = Arc::new(FakeAuthentication::default());
        let devices = Arc::new(FakeDevices::default());
        devices.expect_request("PUT", DEVICE_DEFAULT_AGENT_TARGET.to_owned(), String::new());
        let app = test_router(agents.clone(), authentication.clone(), devices.clone());

        let response = app
            .oneshot(device_default_agent_request(true))
            .await
            .expect("设备默认 Agent 路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["agentId"], AGENT_UUID);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);
        assert_eq!(
            agents.device_default_agent_ensures.load(Ordering::SeqCst),
            1
        );
        assert!(
            authentication
                .requirements
                .lock()
                .expect("认证要求记录锁可用")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn 非_uuidv7_幂等键在认证和业务用例前失败() {
        let agents = Arc::new(FakeAgents::default());
        let authentication = Arc::new(FakeAuthentication::default());
        let app = test_router(
            agents.clone(),
            authentication.clone(),
            Arc::new(FakeDevices::default()),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents")
                    .header(header::ORIGIN, FRONTEND_ORIGIN)
                    .header(header::COOKIE, SESSION_COOKIE)
                    .header("idempotency-key", "550e8400-e29b-41d4-a716-446655440000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "slug": "codex-builder",
                            "displayName": "Codex Builder",
                            "visibility": "private"
                        })
                        .to_string(),
                    ))
                    .expect("非法幂等键请求仍可构造"),
            )
            .await
            .expect("创建 Agent 路由可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            agents
                .creation
                .lock()
                .expect("Agent 创建记录锁可用")
                .is_none()
        );
        assert!(
            authentication
                .requirements
                .lock()
                .expect("认证要求记录锁可用")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn 成员变更强制最近认证() {
        let agents = Arc::new(FakeAgents::default());
        let authentication = Arc::new(FakeAuthentication::default());
        let app = test_router(
            agents.clone(),
            authentication.clone(),
            Arc::new(FakeDevices::default()),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/agents/{AGENT_UUID}/members/{TARGET_PRINCIPAL_UUID}"
                    ))
                    .header(header::ORIGIN, FRONTEND_ORIGIN)
                    .header(header::COOKIE, SESSION_COOKIE)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "role": "operator" }).to_string()))
                    .expect("Agent 成员变更请求有效"),
            )
            .await
            .expect("Agent 成员变更路由可调用");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let changes = agents
            .membership_changes
            .lock()
            .expect("Agent 成员变更记录锁可用");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].agent_id, agent_id());
        assert_eq!(changes[0].principal_id.to_string(), TARGET_PRINCIPAL_UUID);
        assert_eq!(changes[0].role, Some(AgentRole::Operator));
        assert_eq!(
            *authentication
                .requirements
                .lock()
                .expect("认证要求记录锁可用"),
            vec![AuthenticationRequirement::RecentAuthentication]
        );
    }

    #[tokio::test]
    async fn 注册实例按原始正文验证设备证明并仅返回_agent_设备令牌() {
        let agents = Arc::new(FakeAgents::default());
        let devices = Arc::new(FakeDevices::default());
        let public_key = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let body = json!({
            "adapterType": "codex-desktop",
            "capabilityVersion": "2026-08-24",
            "configuration": {
                "capabilities": ["targeted_handoff_v1"]
            },
            "publicSigningKey": public_key
        })
        .to_string();
        devices.expect_request(
            "POST",
            format!("/agents/{AGENT_UUID}/instances"),
            body.clone(),
        );
        let app = test_router(
            agents.clone(),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
        );

        let response = app
            .oneshot(instance_request(&body, true))
            .await
            .expect("Agent 实例注册路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let payload = response_json(response).await;
        assert_eq!(payload["agentId"], AGENT_UUID);
        assert_eq!(payload["displayName"], "Codex Builder");
        assert_eq!(payload["avatarContentId"], Value::Null);
        assert_eq!(payload["matrixDeviceId"], "AR_TEST");
        assert_eq!(payload["accessToken"], "agent-device-access-token");
        assert!(!payload.to_string().contains("application-service-token"));
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);

        let registration = agents
            .registration
            .lock()
            .expect("Agent 实例记录锁可用")
            .clone()
            .expect("Agent 实例注册用例已调用");
        assert_eq!(registration.request_id.to_string(), REQUEST_UUID);
        assert_eq!(registration.agent_id, agent_id());
        assert_eq!(registration.actor.device_id, device_id());
        assert_eq!(registration.adapter_type, "codex-desktop");
        assert_eq!(
            registration.configuration,
            serde_json::Map::from_iter([(
                "capabilities".to_owned(),
                json!(["targeted_handoff_v1"]),
            )])
        );
        assert_eq!(registration.public_signing_key.as_bytes(), &[7_u8; 32]);
    }

    #[tokio::test]
    async fn matrix_会话轮换使用独立设备签名端点而不重放注册正文() {
        let agents = Arc::new(FakeAgents::default());
        let devices = Arc::new(FakeDevices::default());
        let target = format!("/agent-instances/{INSTANCE_UUID}/matrix-session");
        devices.expect_request("POST", target.clone(), String::new());
        let app = test_router(
            agents.clone(),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
        );

        let response = app
            .oneshot(matrix_session_rotation_request(true))
            .await
            .expect("Matrix 会话轮换路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["agentInstanceId"], INSTANCE_UUID);
        assert_eq!(payload["matrixDeviceId"], "AR_TEST");
        assert_eq!(payload["accessToken"], "rotated-agent-device-access-token");
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);
        let rotation = agents
            .rotation
            .lock()
            .expect("Matrix 会话轮换记录锁可用")
            .clone()
            .expect("Matrix 会话轮换用例已调用");
        assert_eq!(rotation.instance_id.to_string(), INSTANCE_UUID);
        assert_eq!(rotation.actor.device_id, device_id());
    }

    #[tokio::test]
    async fn 缺失设备证明时不会触碰认证或_agent_用例() {
        let agents = Arc::new(FakeAgents::default());
        let devices = Arc::new(FakeDevices::default());
        let app = test_router(
            agents.clone(),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
        );
        let body = json!({
            "adapterType": "codex-desktop",
            "capabilityVersion": "2026-08-24",
            "publicSigningKey": URL_SAFE_NO_PAD.encode([7_u8; 32])
        })
        .to_string();

        let response = app
            .oneshot(instance_request(&body, false))
            .await
            .expect("缺失设备证明请求可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 0);
        assert!(
            agents
                .registration
                .lock()
                .expect("Agent 实例记录锁可用")
                .is_none()
        );
    }

    #[tokio::test]
    async fn 查询实例验签材料要求设备持有证明并返回完整时间窗() {
        let devices = Arc::new(FakeDevices::default());
        let verification = Arc::new(FakeVerification::default());
        devices.expect_request(
            "GET",
            format!("/agent-instances/{INSTANCE_UUID}/verification"),
            String::new(),
        );
        let app = test_router_with_verification(
            Arc::new(FakeAgents::default()),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
            verification.clone(),
        );

        let response = app
            .oneshot(verification_request(true))
            .await
            .expect("实例验签材料路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let payload = response_json(response).await;
        assert_eq!(payload["agentInstanceId"], INSTANCE_UUID);
        assert_eq!(payload["agentId"], AGENT_UUID);
        assert_eq!(
            payload["publicSigningKey"],
            URL_SAFE_NO_PAD.encode([11_u8; 32])
        );
        assert_eq!(payload["registeredAtUnixMs"], 1_700_000_000_000_i64);
        assert_eq!(payload["invalidatedAtUnixMs"], 1_700_000_100_000_i64);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);

        let resolutions = verification
            .resolutions
            .lock()
            .expect("实例验签查询记录锁可用");
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].instance_id.to_string(), INSTANCE_UUID);
        assert_eq!(resolutions[0].actor.device_id, device_id());
    }

    #[tokio::test]
    async fn 缺失设备证明时不会查询实例验签材料() {
        let devices = Arc::new(FakeDevices::default());
        let verification = Arc::new(FakeVerification::default());
        let app = test_router_with_verification(
            Arc::new(FakeAgents::default()),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
            verification.clone(),
        );

        let response = app
            .oneshot(verification_request(false))
            .await
            .expect("缺失设备证明的实例验签请求可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 0);
        assert!(
            verification
                .resolutions
                .lock()
                .expect("实例验签查询记录锁可用")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn 未注册配置键不能越过实例登记边界() {
        let agents = Arc::new(FakeAgents::default());
        let devices = Arc::new(FakeDevices::default());
        let body = json!({
            "adapterType": "codex-desktop",
            "capabilityVersion": "2026-08-24",
            "configuration": { "accessToken": "禁止落库" },
            "publicSigningKey": URL_SAFE_NO_PAD.encode([7_u8; 32])
        })
        .to_string();
        devices.expect_request(
            "POST",
            format!("/agents/{AGENT_UUID}/instances"),
            body.clone(),
        );
        let app = test_router(
            agents.clone(),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
        );

        let response = app
            .oneshot(instance_request(&body, true))
            .await
            .expect("带任意适配器配置的请求可调用");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);
        assert!(
            agents
                .registration
                .lock()
                .expect("Agent 实例记录锁可用")
                .is_none()
        );
    }

    #[tokio::test]
    async fn 能力列表类型或内容不规范时拒绝实例登记() {
        for configuration in [
            json!({ "capabilities": "targeted_handoff_v1" }),
            json!({ "capabilities": ["unknown"] }),
            json!({ "capabilities": ["targeted_handoff_v1", "targeted_handoff_v1"] }),
        ] {
            let agents = Arc::new(FakeAgents::default());
            let devices = Arc::new(FakeDevices::default());
            let body = json!({
                "adapterType": "codex-desktop",
                "capabilityVersion": "2026-08-24",
                "configuration": configuration,
                "publicSigningKey": URL_SAFE_NO_PAD.encode([7_u8; 32])
            })
            .to_string();
            devices.expect_request(
                "POST",
                format!("/agents/{AGENT_UUID}/instances"),
                body.clone(),
            );
            let app = test_router(
                agents.clone(),
                Arc::new(FakeAuthentication::default()),
                devices,
            );

            let response = app
                .oneshot(instance_request(&body, true))
                .await
                .expect("非法能力配置请求可调用");

            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(
                agents
                    .registration
                    .lock()
                    .expect("Agent 实例记录锁可用")
                    .is_none()
            );
        }
    }

    fn instance_request(body: &str, include_proof: bool) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/agents/{AGENT_UUID}/instances"))
            .header(header::AUTHORIZATION, "Bearer device-access-token")
            .header("idempotency-key", REQUEST_UUID)
            .header(header::CONTENT_TYPE, "application/json");
        if include_proof {
            request = request
                .header("x-agent-room-device-id", DEVICE_UUID)
                .header("x-agent-room-proof-issued-at", "1700000000000")
                .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
                .header(
                    "x-agent-room-proof-signature",
                    URL_SAFE_NO_PAD.encode([9_u8; 64]),
                );
        }
        request
            .body(Body::from(body.to_owned()))
            .expect("Agent 实例注册请求有效")
    }

    fn device_default_agent_request(include_proof: bool) -> Request<Body> {
        let mut request = Request::builder()
            .method("PUT")
            .uri(DEVICE_DEFAULT_AGENT_TARGET)
            .header(header::AUTHORIZATION, "Bearer device-access-token");
        if include_proof {
            request = request
                .header("x-agent-room-device-id", DEVICE_UUID)
                .header("x-agent-room-proof-issued-at", "1700000000000")
                .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
                .header(
                    "x-agent-room-proof-signature",
                    URL_SAFE_NO_PAD.encode([9_u8; 64]),
                );
        }
        request
            .body(Body::empty())
            .expect("设备默认 Agent 请求有效")
    }

    fn verification_request(include_proof: bool) -> Request<Body> {
        let mut request = Request::builder()
            .method("GET")
            .uri(format!("/agent-instances/{INSTANCE_UUID}/verification"))
            .header(header::AUTHORIZATION, "Bearer device-access-token");
        if include_proof {
            request = request
                .header("x-agent-room-device-id", DEVICE_UUID)
                .header("x-agent-room-proof-issued-at", "1700000000000")
                .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
                .header(
                    "x-agent-room-proof-signature",
                    URL_SAFE_NO_PAD.encode([9_u8; 64]),
                );
        }
        request.body(Body::empty()).expect("实例验签材料请求有效")
    }

    fn matrix_session_rotation_request(include_proof: bool) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/agent-instances/{INSTANCE_UUID}/matrix-session"))
            .header(header::AUTHORIZATION, "Bearer device-access-token");
        if include_proof {
            request = request
                .header("x-agent-room-device-id", DEVICE_UUID)
                .header("x-agent-room-proof-issued-at", "1700000000000")
                .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
                .header(
                    "x-agent-room-proof-signature",
                    URL_SAFE_NO_PAD.encode([9_u8; 64]),
                );
        }
        request
            .body(Body::empty())
            .expect("Matrix 会话轮换请求有效")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&body).expect("响应正文是 JSON")
    }

    fn registered_agent() -> RegisteredAgent {
        RegisteredAgent {
            agent: Agent::register(agent_id()),
            matrix_user_id: matrix_user_id().as_str().to_owned(),
            slug: "codex-builder".to_owned(),
            display_name: "Codex Builder".to_owned(),
            description: "构建 Agent Room".to_owned(),
            avatar_content_id: None,
            visibility: AgentVisibility::Private,
            registered_at: time(1_700_000_000_000),
        }
    }

    fn registered_instance() -> RegisteredAgentInstance {
        let binding_id = AdapterBindingId::from_uuid(uuid(BINDING_UUID));
        let binding = AdapterBinding::register(
            binding_id,
            agent_id(),
            "codex-desktop".to_owned(),
            None,
            "2026-08-24".to_owned(),
        )
        .expect("测试适配器绑定有效");
        let matrix_device_id =
            AgentMatrixDeviceId::new("AR_TEST".to_owned()).expect("测试 Matrix 设备标识有效");
        let instance = AgentInstance::register(
            AgentInstanceId::from_uuid(uuid(INSTANCE_UUID)),
            agent_id(),
            device_id(),
            binding_id,
            AgentInstancePublicSigningKey::new(vec![7; 32]).expect("测试实例公钥有效"),
            matrix_device_id,
        );
        RegisteredAgentInstance {
            agent: registered_agent(),
            registration: StoredAgentInstanceRegistration { binding, instance },
            matrix_session: MatrixSession::new(
                MatrixSessionMetadata::new(
                    matrix_user_id(),
                    MatrixDeviceId::new("AR_TEST").expect("测试 Matrix 设备标识有效"),
                ),
                SecretValue::new("agent-device-access-token").expect("测试设备令牌有效"),
                None,
            ),
        }
    }

    fn rotated_instance() -> RotatedAgentInstanceMatrixSession {
        let registered = registered_instance();
        RotatedAgentInstanceMatrixSession {
            instance: AgentInstanceManagementRecord {
                instance: registered.registration.instance,
                agent_matrix_user_id: matrix_user_id().as_str().to_owned(),
                agent_display_name: "Codex Builder".to_owned(),
                agent_avatar_content_id: None,
                adapter_type: "codex-desktop".to_owned(),
                capability_version: "2026-08-24".to_owned(),
                device_label: "Windows 工作站".to_owned(),
                device_platform: DevicePlatform::Windows,
                device_trust_state: DeviceTrustState::Verified,
                created_at: time(1_700_000_000_000),
                last_seen_at: None,
                revoked_at: None,
                matrix_device_revoked_at: None,
            },
            matrix_session: MatrixSession::new(
                MatrixSessionMetadata::new(
                    matrix_user_id(),
                    MatrixDeviceId::new("AR_TEST").expect("测试 Matrix 设备标识有效"),
                ),
                SecretValue::new("rotated-agent-device-access-token").expect("测试轮换令牌有效"),
                None,
            ),
        }
    }

    fn verification_record() -> AgentInstanceVerificationRecord {
        AgentInstanceVerificationRecord {
            instance_id: AgentInstanceId::from_uuid(uuid(INSTANCE_UUID)),
            agent_id: agent_id(),
            public_signing_key: AgentInstancePublicSigningKey::new(vec![11; 32])
                .expect("测试实例验签公钥有效"),
            registered_at: time(1_700_000_000_000),
            invalidated_at: Some(time(1_700_000_100_000)),
        }
    }

    fn authenticated_principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_id: principal_id(),
            matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            locale: "zh-CN".to_owned(),
            authenticated_at: time(1_700_000_000_000),
            expires_at: time(1_700_028_800_000),
            recently_authenticated: true,
        }
    }

    fn authenticated_device() -> AuthenticatedDevice {
        AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(principal_id()),
                matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
                display_name: "Agent Room User".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: device_id(),
            access_token_expires_at: time(1_700_000_900_000),
        }
    }

    fn matrix_user_id() -> MatrixUserId {
        MatrixUserId::new(format!(
            "@_agent_{}:matrix.agent-room.test",
            agent_id().as_uuid().simple()
        ))
        .expect("测试 Matrix 用户标识有效")
    }

    fn principal_id() -> PrincipalId {
        PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))
    }

    fn device_id() -> DeviceId {
        DeviceId::from_uuid(uuid(DEVICE_UUID))
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(uuid(AGENT_UUID))
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
