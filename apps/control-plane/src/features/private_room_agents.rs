//! 私人房间的 Agent 口令：房主或管理员在网页上查看、生成、停用口令并移出凭口令进来的 Agent；
//! 本机 Agent 用设备签名的请求凭口令加入。口令本身只在生成的那次响应里出现。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationUseCases},
    devices::DeviceAuthorizationUseCases,
    ports::{PrivateRoomAgentMemberRecord, SecretFactory},
    private_rooms::{
        AgentAccessView, GeneratedJoinCode, InspectAgentAccess, ManageJoinCode,
        PrivateRoomAgentAccessUseCases, RedeemJoinCode, RedeemedRoom, RemoveAgentMember,
    },
};
use agent_room_domain::ids::{AgentId, AgentInstanceId, RoomCatalogId};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const MAX_AGENT_ACCESS_BODY_BYTES: usize = 4 * 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomAgentHttpState {
    access: Arc<dyn PrivateRoomAgentAccessUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct PrivateRoomAgentHttpDependencies {
    pub(crate) access: Arc<dyn PrivateRoomAgentAccessUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl PrivateRoomAgentHttpState {
    pub(crate) fn new(
        dependencies: PrivateRoomAgentHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            access: dependencies.access,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: PrivateRoomAgentHttpState) -> Router {
    Router::new()
        .route("/private-rooms/{catalog_id}/agent-access", get(inspect))
        .route(
            "/private-rooms/{catalog_id}/agent-access/code",
            put(generate_code).delete(disable_code),
        )
        .route(
            "/private-rooms/{catalog_id}/agent-access/agents/{agent_id}",
            delete(remove_agent),
        )
        .route(
            "/agents/{agent_id}/instances/{instance_id}/join-codes/redeem",
            post(redeem),
        )
        .layer(DefaultBodyLimit::max(MAX_AGENT_ACCESS_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentAccessResponse {
    /// 口令的元数据；口令本身只在生成时返回一次。
    join_code: Option<JoinCodeSummary>,
    agents: Vec<AgentMemberResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinCodeSummary {
    created_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentMemberResponse {
    agent_id: String,
    display_name: String,
    owner_display_name: Option<String>,
    status: &'static str,
    joined_at_unix_ms: i64,
    status_changed_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedCodeResponse {
    /// 给人看的形式 `XXXX-XXXX-XXXX`。
    code: String,
    created_at_unix_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RedeemBody {
    code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RedeemedRoomResponse {
    catalog_id: String,
    matrix_room_id: String,
    name: String,
}

impl From<AgentAccessView> for AgentAccessResponse {
    fn from(view: AgentAccessView) -> Self {
        Self {
            join_code: view.join_code.map(|record| JoinCodeSummary {
                created_at_unix_ms: record.created_at.value(),
            }),
            agents: view
                .agents
                .into_iter()
                .map(AgentMemberResponse::from)
                .collect(),
        }
    }
}

impl From<PrivateRoomAgentMemberRecord> for AgentMemberResponse {
    fn from(record: PrivateRoomAgentMemberRecord) -> Self {
        Self {
            agent_id: record.agent_id.to_string(),
            display_name: record.display_name,
            owner_display_name: record.owner_display_name,
            status: record.status.as_str(),
            joined_at_unix_ms: record.joined_at.value(),
            status_changed_at_unix_ms: record.status_changed_at.value(),
        }
    }
}

impl From<GeneratedJoinCode> for GeneratedCodeResponse {
    fn from(generated: GeneratedJoinCode) -> Self {
        Self {
            code: generated.code.display(),
            created_at_unix_ms: generated.record.created_at.value(),
        }
    }
}

impl From<RedeemedRoom> for RedeemedRoomResponse {
    fn from(room: RedeemedRoom) -> Self {
        Self {
            catalog_id: room.catalog_id.to_string(),
            matrix_room_id: room.matrix_room_id.as_str().to_owned(),
            name: room.name,
        }
    }
}

async fn inspect(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Ok(catalog_id) = parse_uuid_v7(&catalog).map(RoomCatalogId::from_uuid) else {
        return invalid_resource(correlation_id);
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
        .access
        .inspect(InspectAgentAccess { actor, catalog_id })
        .await
    {
        Ok(view) => {
            no_store((StatusCode::OK, Json(AgentAccessResponse::from(view))).into_response())
        }
        Err(failure) => no_store(ApiError::agent_access(failure, correlation_id).into_response()),
    }
}

async fn generate_code(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, catalog_id) =
        match write_context(&state, &headers, &jar, &catalog, correlation_id).await {
            Ok(context) => context,
            Err(response) => return response,
        };
    match state
        .access
        .generate_code(ManageJoinCode { actor, catalog_id })
        .await
    {
        Ok(generated) => {
            no_store((StatusCode::OK, Json(GeneratedCodeResponse::from(generated))).into_response())
        }
        Err(failure) => no_store(ApiError::agent_access(failure, correlation_id).into_response()),
    }
}

async fn disable_code(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, catalog_id) =
        match write_context(&state, &headers, &jar, &catalog, correlation_id).await {
            Ok(context) => context,
            Err(response) => return response,
        };
    empty(
        state
            .access
            .disable_code(ManageJoinCode { actor, catalog_id })
            .await,
        correlation_id,
    )
}

async fn remove_agent(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, agent)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let Ok(agent_id) = parse_uuid_v7(&agent).map(AgentId::from_uuid) else {
        return invalid_resource(correlation_id);
    };
    let (actor, catalog_id) =
        match write_context(&state, &headers, &jar, &catalog, correlation_id).await {
            Ok(context) => context,
            Err(response) => return response,
        };
    empty(
        state
            .access
            .remove_agent(RemoveAgentMember {
                actor,
                catalog_id,
                agent_id,
            })
            .await,
        correlation_id,
    )
}

async fn redeem(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((agent, instance)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_target = format!("/agents/{agent}/instances/{instance}/join-codes/redeem");
    let Ok(agent_id) = parse_uuid_v7(&agent).map(AgentId::from_uuid) else {
        return invalid_resource(correlation_id);
    };
    let Ok(agent_instance_id) = parse_uuid_v7(&instance).map(AgentInstanceId::from_uuid) else {
        return invalid_resource(correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid_body(correlation_id);
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
    let Ok(body) = serde_json::from_slice::<RedeemBody>(&body) else {
        return invalid_body(correlation_id);
    };
    match state
        .access
        .redeem(RedeemJoinCode {
            actor,
            agent_id,
            agent_instance_id,
            code: body.code,
        })
        .await
    {
        Ok(room) => {
            no_store((StatusCode::OK, Json(RedeemedRoomResponse::from(room))).into_response())
        }
        Err(failure) => no_store(ApiError::agent_access(failure, correlation_id).into_response()),
    }
}

/// 写操作：来源必须是本站或桌面端，会话有效，房间 ID 合法。
async fn write_context(
    state: &PrivateRoomAgentHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    catalog: &str,
    correlation_id: CorrelationId,
) -> Result<(AuthenticatedPrincipal, RoomCatalogId), Response> {
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "join_code.invalid_origin",
                ErrorCategory::Authorization,
                "Agent 口令写请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    let Ok(catalog_id) = parse_uuid_v7(catalog).map(RoomCatalogId::from_uuid) else {
        return Err(invalid_resource(correlation_id));
    };
    let actor = authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await?;
    Ok((actor, catalog_id))
}

fn empty(
    result: agent_room_application::private_rooms::AgentAccessResult<()>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::agent_access(failure, correlation_id).into_response()),
    }
}

fn invalid_resource(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::invalid_request("join_code.invalid_resource_id", correlation_id).into_response(),
    )
}

fn invalid_body(correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request("join_code.invalid_body", correlation_id).into_response())
}

#[cfg(test)]
mod tests;
