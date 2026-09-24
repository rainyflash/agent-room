//! 只凭网络接入的 Agent（ADR 0010、`specs/network-agents/design.md`）：不装应用、不用 CLI，
//! 发一个 HTTP 请求起名并进公开大厅。除创建外都用创建时拿到的令牌认证。
//!
//! 这些路由不用 Cookie、不经过设备签名，所以在控制面带凭据的 CORS 之外单独合并，
//! 允许任何来源、不带凭据。

use std::{net::IpAddr, sync::Arc};

use agent_room_application::{
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentUseCases, NetworkAgentView,
    },
    ports::Clock,
};
use agent_room_identity_adapter::NetworkSourceDigester;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderName, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

use crate::{
    correlation::{CORRELATION_ID_HEADER, CorrelationId},
    error::ApiError,
    features::{authentication::no_store, devices::bearer_secret},
};

const MAX_NETWORK_AGENT_BODY_BYTES: usize = 4 * 1_024;
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1_000;
const SCHEMA_VERSION: u8 = 1;

#[derive(Clone)]
pub(crate) struct NetworkAgentHttpState {
    pub(crate) agents: Arc<dyn NetworkAgentUseCases>,
    pub(crate) sources: Arc<NetworkSourceDigester>,
    pub(crate) clock: Arc<dyn Clock>,
}

pub(crate) fn router(state: NetworkAgentHttpState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([
            header::RETRY_AFTER,
            HeaderName::from_static(CORRELATION_ID_HEADER),
        ]);
    Router::new()
        .route("/v1/network-agents", post(create))
        .route("/v1/network-agents/me", get(me).delete(disable))
        .layer(DefaultBodyLimit::max(MAX_NETWORK_AGENT_BODY_BYTES))
        .layer(cors)
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateBody {
    name: String,
    /// 公开大厅的名字或 slug；省略就进默认公开大厅。
    #[serde(default)]
    room: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedResponse {
    schema_version: u8,
    agent_id: String,
    /// 实际用的名字：同名时带序号。
    display_name: String,
    /// 只在这一次返回；之后的请求都带 `Authorization: Bearer <token>`。
    token: String,
    room: RoomResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomResponse {
    catalog_id: String,
    matrix_room_id: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeResponse {
    schema_version: u8,
    agent_id: String,
    display_name: String,
    created_at_unix_ms: i64,
}

impl From<CreatedNetworkAgent> for CreatedResponse {
    fn from(created: CreatedNetworkAgent) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            agent_id: created.agent_id.to_string(),
            display_name: created.display_name,
            token: created.token.expose().to_owned(),
            room: RoomResponse {
                catalog_id: created.room.catalog_id.to_string(),
                matrix_room_id: created.room.matrix_room_id.as_str().to_owned(),
                name: created.room.name,
            },
        }
    }
}

impl From<NetworkAgentView> for MeResponse {
    fn from(view: NetworkAgentView) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            agent_id: view.agent_id.to_string(),
            display_name: view.display_name,
            created_at_unix_ms: view.created_at.value(),
        }
    }
}

async fn create(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_request",
                ErrorCategory::Validation,
                "请求体应为 JSON 对象：{\"name\": 名字, \"room\": 可选的公开大厅名}。",
                correlation_id,
            )
            .into_response(),
        );
    };
    let request = CreateNetworkAgent {
        name: body.name,
        room: body.room,
        source_digest: state.source_digest(&headers),
    };
    match state.agents.create(request).await {
        Ok(created) => {
            no_store((StatusCode::CREATED, Json(CreatedResponse::from(created))).into_response())
        }
        Err(failure) => no_store(ApiError::network_agent(&failure, correlation_id).into_response()),
    }
}

async fn me(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    // 没带令牌也交给用例：总开关关着时统一回答“已关闭”，开着时才是“未认证”。
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.agents.me(token).await {
        Ok(view) => no_store((StatusCode::OK, Json(MeResponse::from(view))).into_response()),
        Err(failure) => no_store(ApiError::network_agent(&failure, correlation_id).into_response()),
    }
}

/// 停用：令牌立即作废。重复停用、令牌已作废都不会再成功，只会得到“未认证”。
async fn disable(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.agents.disable(token).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::network_agent(&failure, correlation_id).into_response()),
    }
}

impl NetworkAgentHttpState {
    /// 来源地址按 UTC 日加盐后的摘要，只用于限流。
    fn source_digest(&self, headers: &HeaderMap) -> [u8; 32] {
        let day = self.clock.now().value().div_euclid(DAY_MILLIS);
        self.sources
            .digest(&client_source(headers), &day.to_string())
    }
}

/// Caddy 不信任上游，自己把真实地址写进 `X-Forwarded-For`；取最后一个值，也就是离控制面最近的
/// 那一跳写入的。IPv6 按 /64 归并：同一网段里换地址绕不过限流。取不到地址时所有这类请求共用
/// 一个来源（只在本地开发时出现）。
fn client_source(headers: &HeaderMap) -> String {
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .rfind(|value| !value.is_empty());
    match forwarded.and_then(|value| value.parse::<IpAddr>().ok()) {
        Some(IpAddr::V4(address)) => address.to_string(),
        Some(IpAddr::V6(address)) => address.to_ipv4_mapped().map_or_else(
            || {
                let [a, b, c, d, ..] = address.segments();
                format!("{a:x}:{b:x}:{c:x}:{d:x}::/64")
            },
            |mapped| mapped.to_string(),
        ),
        None => "unknown".to_owned(),
    }
}

#[cfg(test)]
pub(crate) mod tests;
