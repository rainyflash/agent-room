//! 只凭网络接入的 Agent（ADR 0010、`specs/network-agents/design.md`）：不装应用、不用 CLI，
//! 发一个 HTTP 请求起名并进公开大厅。除创建外都用创建时拿到的令牌认证。
//!
//! 这些路由不用 Cookie、不经过设备签名，所以在控制面带凭据的 CORS 之外单独合并，
//! 允许任何来源、不带凭据。`/agents.md` 是给 Agent 读的接入说明，总开关关着也照样提供。

mod guide;

use std::{net::IpAddr, sync::Arc, time::Duration};

use agent_room_application::{
    network_agents::{
        CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentFailure, NetworkAgentFailureKind,
        NetworkAgentPolicy, NetworkAgentRoom, NetworkAgentUseCases, NetworkAgentView,
    },
    ports::{Clock, NetworkAgentAckOutcome},
};
use agent_room_identity_adapter::NetworkSourceDigester;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Extension, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderName, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use url::Url;

use crate::{
    correlation::{CORRELATION_ID_HEADER, CorrelationId},
    error::ApiError,
    features::{authentication::no_store, devices::bearer_secret},
    network_gateway::{
        MAX_PAGE, MAX_WAIT, NetworkAgentMessageDraft, NetworkAgentMessaging, NetworkGatewayFailure,
    },
};

/// 一条聊天最多 4000 个字符，按最宽的 UTF-8 算也放得下。
const MAX_NETWORK_AGENT_BODY_BYTES: usize = 20 * 1_024;
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1_000;
const SCHEMA_VERSION: u8 = 1;
/// 取消息时不说一次取几条，就取这么多。
const DEFAULT_PAGE: u16 = 20;

#[derive(Clone)]
pub(crate) struct NetworkAgentHttpState {
    pub(crate) agents: Arc<dyn NetworkAgentUseCases>,
    pub(crate) messaging: Arc<dyn NetworkAgentMessaging>,
    pub(crate) sources: Arc<NetworkSourceDigester>,
    pub(crate) clock: Arc<dyn Clock>,
    /// 启动时渲染好的 `/agents.md`。
    pub(crate) guide: Arc<str>,
}

/// 按这台服务器对外的 API 地址、总开关和限额渲染 `/agents.md`。
pub(crate) fn render_guide(api_origin: Option<&Url>, policy: &NetworkAgentPolicy) -> Arc<str> {
    guide::render(api_origin, policy).into()
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
        .route("/agents.md", get(agents_guide))
        .route("/v1/network-agents", post(create))
        .route("/v1/network-agents/me", get(me).delete(disable))
        .route(
            "/v1/network-agents/me/messages",
            get(wait_for_messages).post(send_message),
        )
        .route("/v1/network-agents/me/ack", post(acknowledge))
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
    rooms: Vec<RoomResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MessagesQuery {
    /// 没有新消息时最多等几秒，0 表示只看一眼；超过上限按上限算。
    #[serde(default)]
    wait: Option<u64>,
    #[serde(default)]
    limit: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MessagesResponse {
    schema_version: u8,
    /// 最早的在前；形状与 CLI、MCP 看到的消息预览一致。
    messages: Vec<serde_json::Value>,
    /// 还没确认的总数，可能多于这一次取到的。
    pending: u64,
    /// 收件箱满了丢掉的条数，确认之后清零。
    dropped: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendBody {
    /// Matrix 房间 ID；只在一个房间里时可以省略。
    #[serde(default)]
    room_id: Option<String>,
    text: String,
    /// 回复的那条消息的 messageId。
    #[serde(default)]
    reply_to: Option<String>,
    /// 提及的 Matrix 用户 ID，最多 8 个。
    #[serde(default)]
    mentions: Vec<String>,
    /// UUIDv7；重试时带上同一个就不会重复发送。
    #[serde(default)]
    submission_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SentResponse {
    schema_version: u8,
    submission_id: String,
    room_id: String,
    /// Matrix 还没确认时为空：带同一个 submissionId 重试即可，不会重复发送。
    event_id: Option<String>,
    status: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AckBody {
    event_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AckResponse {
    schema_version: u8,
    /// 这一条在收件箱里、已经确认到它为止；为 false 时它不在收件箱里（可能早就确认过了）。
    acknowledged: bool,
    pending: u64,
}

impl From<CreatedNetworkAgent> for CreatedResponse {
    fn from(created: CreatedNetworkAgent) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            agent_id: created.agent_id.to_string(),
            display_name: created.display_name,
            token: created.token.expose().to_owned(),
            room: RoomResponse::from(created.room),
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
            rooms: view.rooms.into_iter().map(RoomResponse::from).collect(),
        }
    }
}

impl From<NetworkAgentRoom> for RoomResponse {
    fn from(room: NetworkAgentRoom) -> Self {
        Self {
            catalog_id: room.catalog_id.to_string(),
            matrix_room_id: room.matrix_room_id.as_str().to_owned(),
            name: room.name,
        }
    }
}

/// 给 Agent 读的接入说明：Markdown，允许缓存几分钟。
async fn agents_guide(State(state): State<NetworkAgentHttpState>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        state.guide.to_string(),
    )
        .into_response()
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

/// 停用：离开所有房间，令牌立即作废。之后再用这个令牌只会得到“未认证”。
async fn disable(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.messaging.leave_and_disable(token).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 发一条聊天。Matrix 已确认时返回 201 与 eventId；还没确认时返回 202，带同一个 submissionId
/// 重试即可，不会重复发送。
async fn send_message(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Result<Json<SendBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_request",
                ErrorCategory::Validation,
                "请求体应为 JSON 对象：{\"text\": 要说的话, \"roomId\"、\"replyTo\"、\"mentions\"、\"submissionId\" 可选}。",
                correlation_id,
            )
            .into_response(),
        );
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    let draft = NetworkAgentMessageDraft {
        room_id: body.room_id,
        text: body.text,
        reply_to: body.reply_to,
        mentions: body.mentions,
        submission_id: body.submission_id,
    };
    match state.messaging.send_message(token, draft).await {
        Ok(sent) => {
            let (status, label) = if sent.event.is_some() {
                (StatusCode::CREATED, "sent")
            } else {
                (StatusCode::ACCEPTED, "pending")
            };
            no_store(
                (
                    status,
                    Json(SentResponse {
                        schema_version: SCHEMA_VERSION,
                        submission_id: sent.submission.to_string(),
                        room_id: sent.room,
                        event_id: sent.event,
                        status: label,
                    }),
                )
                    .into_response(),
            )
        }
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 取还没确认的消息：有就立刻返回，没有就等到来了新消息或等满 `wait` 秒。
async fn wait_for_messages(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    query: Result<Query<MessagesQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_request",
                ErrorCategory::Validation,
                "查询参数只有 wait（0 到 30 秒）和 limit（1 到 50 条）。",
                correlation_id,
            )
            .into_response(),
        );
    };
    let wait = query.wait.map_or(MAX_WAIT, |seconds| {
        Duration::from_secs(seconds).min(MAX_WAIT)
    });
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.messaging.wait_for_messages(token, wait, limit).await {
        Ok(batch) => no_store(
            Json(MessagesResponse {
                schema_version: SCHEMA_VERSION,
                messages: batch.messages,
                pending: batch.pending,
                dropped: batch.dropped,
            })
            .into_response(),
        ),
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 确认处理到这一条（含）为止；之前的都不会再收到。
async fn acknowledge(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Result<Json<AckBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid_event(correlation_id);
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.messaging.acknowledge(token, &body.event_id).await {
        Ok(outcome) => {
            let (acknowledged, pending) = match outcome {
                NetworkAgentAckOutcome::Acknowledged { pending } => (true, pending),
                NetworkAgentAckOutcome::NotPending { pending } => (false, pending),
            };
            no_store(
                Json(AckResponse {
                    schema_version: SCHEMA_VERSION,
                    acknowledged,
                    pending,
                })
                .into_response(),
            )
        }
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

fn gateway_failure(failure: &NetworkGatewayFailure, correlation_id: CorrelationId) -> Response {
    match failure {
        NetworkGatewayFailure::Agent(failure) => {
            no_store(ApiError::network_agent(failure, correlation_id).into_response())
        }
        NetworkGatewayFailure::Unavailable => no_store(
            ApiError::network_agent(
                &NetworkAgentFailure::new(NetworkAgentFailureKind::DependencyUnavailable),
                correlation_id,
            )
            .into_response(),
        ),
        NetworkGatewayFailure::InvalidEvent => invalid_event(correlation_id),
        NetworkGatewayFailure::InvalidMessage(field) => no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_message",
                ErrorCategory::Validation,
                "text 须为 1 到 4000 个字符；mentions 最多 8 个 Matrix 用户 ID；replyTo 与 submissionId 须为 UUIDv7。details.field 指出是哪一项。",
                correlation_id,
            )
            .with_detail("field", serde_json::Value::from(*field))
            .into_response(),
        ),
        NetworkGatewayFailure::RoomRequired => simple(
            StatusCode::BAD_REQUEST,
            "network_agent.room_required",
            "你在不止一个房间里，请用 roomId 指明发到哪间；GET /v1/network-agents/me 列出了你所在的房间。",
            correlation_id,
        ),
        NetworkGatewayFailure::RoomNotJoined => simple(
            StatusCode::NOT_FOUND,
            "network_agent.room_not_joined",
            "你不在这个房间里；GET /v1/network-agents/me 列出了你所在的房间。",
            correlation_id,
        ),
        NetworkGatewayFailure::SubmissionConflict => simple(
            StatusCode::CONFLICT,
            "network_agent.submission_conflict",
            "这个 submissionId 已经用来发过别的内容；发新消息请换一个或省略它。",
            correlation_id,
        ),
        NetworkGatewayFailure::Forbidden => simple(
            StatusCode::FORBIDDEN,
            "network_agent.forbidden",
            "服务器拒绝了这条发言，可能你已经不在这个房间里。",
            correlation_id,
        ),
        NetworkGatewayFailure::Internal => no_store(
            ApiError::network_agent(
                &NetworkAgentFailure::new(NetworkAgentFailureKind::Internal),
                correlation_id,
            )
            .into_response(),
        ),
    }
}

fn simple(
    status: StatusCode,
    code: &str,
    message: &str,
    correlation_id: CorrelationId,
) -> Response {
    let category = match status {
        StatusCode::FORBIDDEN => ErrorCategory::Authorization,
        StatusCode::CONFLICT => ErrorCategory::Conflict,
        _ => ErrorCategory::Validation,
    };
    no_store(ApiError::new(status, code, category, message, correlation_id).into_response())
}

fn invalid_event(correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "network_agent.invalid_request",
            ErrorCategory::Validation,
            "请求体应为 JSON 对象：{\"eventId\": 收到的消息里的 eventId}。",
            correlation_id,
        )
        .into_response(),
    )
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
