//! 按需查看的 HTTP 接口（`specs/agent-reading/design.md` 第 5 步）：按 ID 取、看前后、往前翻。
//! 都不动收件箱，只给它所在房间里的消息。

use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json,
    extract::{Extension, Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{NetworkAgentHttpState, SCHEMA_VERSION, gateway_failure};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{authentication::no_store, devices::bearer_secret},
    network_gateway::{NetworkAgentRoomMessagesRequest, NetworkAgentRoomQuery},
};

/// 看前后、往前翻默认给几条；HTTP 接口与远程 MCP 共用。
pub(super) const DEFAULT_VIEW_LIMIT: u16 = 20;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LookupQuery {
    /// 逗号分隔的事件 ID 或消息 ID，1 到 20 个。
    ids: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMessagesQuery {
    /// 看这条（事件 ID 或消息 ID）和它前后的消息；不能再给 before、after、from、mentionsMe。
    #[serde(default)]
    around: Option<String>,
    /// 从这条往前翻，新的在前。
    #[serde(default)]
    before: Option<String>,
    /// 从这条往后翻，旧的在前。
    #[serde(default)]
    after: Option<String>,
    /// 1 到 50，默认 20。
    #[serde(default)]
    limit: Option<u16>,
    /// 只看某个人：Matrix 用户 ID 或名字。
    #[serde(default)]
    from: Option<String>,
    /// 只看提到你或回复你的。
    #[serde(default)]
    mentions_me: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FoundResponse {
    schema_version: u8,
    /// 按要的顺序，每条都是全文。
    messages: Vec<Value>,
    /// 找不到的，或者不在你所在房间里的。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomMessagesResponse {
    schema_version: u8,
    messages: Vec<Value>,
    /// 接着翻的位置：往前翻时当 before、往后翻时当 after 再取；没有就是翻到头了。
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

/// 按 ID 取：`GET /v1/network-agents/me/messages/lookup?ids=…`。
pub(super) async fn get_messages(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    query: Result<Query<LookupQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(invalid_lookup_error("ids", correlation_id).into_response());
    };
    let ids: Vec<String> = query
        .ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.messaging.get_messages(token, ids).await {
        Ok(found) => no_store(
            Json(FoundResponse {
                schema_version: SCHEMA_VERSION,
                messages: found.messages,
                missing: found.missing,
            })
            .into_response(),
        ),
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 看前后、往前翻：`GET /v1/network-agents/me/rooms/{roomId}/messages`。
pub(super) async fn room_messages(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    query: Result<Query<RoomMessagesQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return no_store(invalid_lookup_error("query", correlation_id).into_response());
    };
    let query = match NetworkAgentRoomQuery::parse(
        query.around,
        query.before,
        query.after,
        query.limit.unwrap_or(DEFAULT_VIEW_LIMIT),
        query.from,
        query.mentions_me.unwrap_or(false),
    ) {
        Ok(query) => query,
        Err(field) => return no_store(invalid_lookup_error(field, correlation_id).into_response()),
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    let request = NetworkAgentRoomMessagesRequest {
        room: Some(room_id),
        query,
    };
    match state.messaging.room_messages(token, request).await {
        Ok(page) => no_store(
            Json(RoomMessagesResponse {
                schema_version: SCHEMA_VERSION,
                messages: page.messages,
                next_cursor: page.next_cursor,
            })
            .into_response(),
        ),
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 按需查看的参数不对；HTTP 接口与远程 MCP 共用。
pub(super) fn invalid_lookup_error(field: &'static str, correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "network_agent.invalid_request",
        ErrorCategory::Validation,
        "Invalid lookup parameters: ids is 1 to 20 event IDs (starting with $) or message IDs, separated by commas; around, before and after each take one message's event ID or message ID; around can't be combined with before, after, from or mentionsMe, and before and after can't both be given; limit is 1 to 50; from is a Matrix user ID or a name; mentionsMe is true or false. details.field says which one is wrong.",
        correlation_id,
    )
    .with_detail("field", Value::from(field))
}
