//! 用网络 Agent 自己的 Matrix 会话访问 Matrix（ADR 0010，2-收发）。访问令牌由服务器封存保管，
//! 每次调用时解封传入。这里只发受限的客户端—服务器请求，不上传端到端加密密钥：公开大厅不加密。

use std::{collections::BTreeMap, time::Duration};

use agent_room_application::ports::{
    MatrixAcceptedEvent, MatrixBackfillToken, MatrixEvent, MatrixEventId, MatrixEventType,
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, MatrixRoomSync,
    MatrixRoomSyncKind, MatrixStateEvent, MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent,
    MatrixTransactionId, MatrixUserId, NetworkAgentMatrixGateway, NetworkAgentSyncRequest,
    PortFuture, SecretValue,
};
use reqwest::{Client, Url, redirect::Policy};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    MatrixApplicationServiceConfigurationError, decode_json, decode_matrix_error, invalid_response,
    map_matrix_error, map_transport_error, read_body_within, read_limited_body,
    validate_homeserver_url,
};

/// 一次同步最多读这么多字节；时间线按条数限量，正常远小于这个数。
const MAX_SYNC_RESPONSE_BYTES: usize = 4 * 1_024 * 1_024;
/// 长轮询最多等这么久，与网络 Agent 接口的上限一致。
const MAX_SYNC_TIMEOUT: Duration = Duration::from_secs(30);
/// 长轮询之外留给网络与服务器处理的余量。
const REQUEST_MARGIN: Duration = Duration::from_secs(15);
/// 只同步 Agent Room 的消息与修订；v2 是网页里的人发的。
const MESSAGE_EVENT_TYPES: [&str; 4] = [
    "io.github.rainyflash.agentroom.message.preview.v1",
    "io.github.rainyflash.agentroom.message.revision.v1",
    "io.github.rainyflash.agentroom.message.preview.v2",
    "io.github.rainyflash.agentroom.message.revision.v2",
];

/// 以网络 Agent 自己的访问令牌调用 Matrix 客户端—服务器接口。
#[derive(Clone)]
pub struct MatrixAgentSessionClient {
    client: Client,
    homeserver_url: Url,
}

impl MatrixAgentSessionClient {
    /// 创建不跟随重定向、请求期限覆盖一次长轮询的客户端。
    ///
    /// # Errors
    ///
    /// Homeserver 地址无效、生产环境没用 HTTPS 或 HTTP 客户端无法创建时返回错误。
    pub fn new(
        homeserver_url: impl AsRef<str>,
        connect_timeout: Duration,
    ) -> Result<Self, MatrixApplicationServiceConfigurationError> {
        let homeserver_url = Url::parse(homeserver_url.as_ref())
            .map_err(|_| MatrixApplicationServiceConfigurationError::InvalidHomeserverUrl)?;
        validate_homeserver_url(&homeserver_url)?;
        if connect_timeout.is_zero() || connect_timeout > REQUEST_MARGIN {
            return Err(MatrixApplicationServiceConfigurationError::InvalidRequestTimeout);
        }
        let client = Client::builder()
            .timeout(MAX_SYNC_TIMEOUT + REQUEST_MARGIN)
            .connect_timeout(connect_timeout)
            .redirect(Policy::none())
            .build()
            .map_err(|_| MatrixApplicationServiceConfigurationError::HttpClient)?;
        Ok(Self {
            client,
            homeserver_url,
        })
    }

    async fn sync_internal(
        &self,
        access_token: &SecretValue,
        request: &NetworkAgentSyncRequest,
    ) -> MatrixResult<MatrixSyncBatch> {
        let operation = MatrixOperation::Sync;
        let mut url = self
            .homeserver_url
            .join("_matrix/client/v3/sync")
            .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?;
        let timeout = u128::from(request.timeout_millis).min(MAX_SYNC_TIMEOUT.as_millis());
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("filter", &sync_filter(request.timeline_limit).to_string());
            query.append_pair("timeout", &timeout.to_string());
            // Agent Room 用自己的状态事件表达在线，同步本身不该把 Matrix 用户标成在线。
            query.append_pair("set_presence", "offline");
            if let Some(since) = &request.since {
                query.append_pair("since", since.as_str());
            }
        }
        let response = self
            .client
            .get(url)
            .bearer_auth(access_token.expose())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_body_within(response, operation, MAX_SYNC_RESPONSE_BYTES).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        let response: SyncResponse = decode_json(&body, operation)?;
        sync_batch(response, operation)
    }
}

impl MatrixAgentSessionClient {
    /// 房间 ID、事件类型、事务 ID 都按路径段编码，不会拼出别的路径。
    fn room_endpoint(
        &self,
        room_id: &MatrixRoomId,
        tail: &[&str],
        operation: MatrixOperation,
    ) -> MatrixResult<Url> {
        let mut url = self
            .homeserver_url
            .join("_matrix/client/v3/rooms/")
            .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?;
        url.path_segments_mut()
            .map_err(|()| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?
            .pop_if_empty()
            .push(room_id.as_str())
            .extend(tail);
        Ok(url)
    }

    async fn send_event_internal(
        &self,
        access_token: &SecretValue,
        room_id: &MatrixRoomId,
        event: &MatrixEvent,
    ) -> MatrixResult<MatrixAcceptedEvent> {
        let operation = MatrixOperation::SendEvent;
        let url = self.room_endpoint(
            room_id,
            &[
                "send",
                event.event_type().as_str(),
                event.transaction_id().as_str(),
            ],
            operation,
        )?;
        let response = self
            .client
            .put(url)
            .bearer_auth(access_token.expose())
            .json(event.content())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        let accepted: EventIdResponse = decode_json(&body, operation)?;
        Ok(MatrixAcceptedEvent::new(
            event.transaction_id().clone(),
            MatrixEventId::new(accepted.event_id).map_err(|_| invalid_response(operation))?,
        ))
    }

    async fn send_state_event_internal(
        &self,
        access_token: &SecretValue,
        room_id: &MatrixRoomId,
        event: &MatrixStateEvent,
    ) -> MatrixResult<MatrixEventId> {
        let operation = MatrixOperation::SendStateEvent;
        let url = self.room_endpoint(
            room_id,
            &[
                "state",
                event.event_type().as_str(),
                event.state_key().as_str(),
            ],
            operation,
        )?;
        let response = self
            .client
            .put(url)
            .bearer_auth(access_token.expose())
            .json(event.content())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        let accepted: EventIdResponse = decode_json(&body, operation)?;
        MatrixEventId::new(accepted.event_id).map_err(|_| invalid_response(operation))
    }

    async fn leave_internal(
        &self,
        access_token: &SecretValue,
        room_id: &MatrixRoomId,
    ) -> MatrixResult<()> {
        let operation = MatrixOperation::Leave;
        let url = self.room_endpoint(room_id, &["leave"], operation)?;
        let response = self
            .client
            .post(url)
            .bearer_auth(access_token.expose())
            .json(&json!({}))
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            return Ok(());
        }
        let error = decode_matrix_error(&body, operation)?;
        let failure = map_matrix_error(operation, status, &error);
        // 已经不在房间里（或房间已不存在）也算离开了。
        if matches!(
            failure.kind(),
            MatrixFailureKind::Forbidden | MatrixFailureKind::NotFound
        ) {
            return Ok(());
        }
        Err(failure)
    }
}

impl NetworkAgentMatrixGateway for MatrixAgentSessionClient {
    fn sync<'a>(
        &'a self,
        access_token: &'a SecretValue,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>> {
        Box::pin(self.sync_internal(access_token, request))
    }

    fn send_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        Box::pin(self.send_event_internal(access_token, room_id, event))
    }

    fn send_state_event<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(self.send_state_event_internal(access_token, room_id, event))
    }

    fn leave<'a>(
        &'a self,
        access_token: &'a SecretValue,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.leave_internal(access_token, room_id))
    }
}

#[derive(Deserialize)]
struct EventIdResponse {
    event_id: String,
}

/// 只要已加入房间里的消息事件：不要状态、回执、输入提示、账户数据和在线信息。
fn sync_filter(timeline_limit: u16) -> Value {
    json!({
        "presence": { "types": [] },
        "account_data": { "types": [] },
        "room": {
            "include_leave": false,
            "state": { "types": [] },
            "ephemeral": { "types": [] },
            "account_data": { "types": [] },
            "timeline": {
                "limit": timeline_limit.max(1),
                "types": MESSAGE_EVENT_TYPES,
            },
        },
    })
}

#[derive(Deserialize)]
struct SyncResponse {
    next_batch: String,
    #[serde(default)]
    rooms: SyncRooms,
}

#[derive(Default, Deserialize)]
struct SyncRooms {
    #[serde(default)]
    join: BTreeMap<String, JoinedRoom>,
}

#[derive(Deserialize)]
struct JoinedRoom {
    #[serde(default)]
    timeline: Timeline,
}

#[derive(Default, Deserialize)]
struct Timeline {
    #[serde(default)]
    events: Vec<Value>,
    #[serde(default)]
    limited: bool,
    #[serde(default)]
    prev_batch: Option<String>,
}

fn sync_batch(response: SyncResponse, operation: MatrixOperation) -> MatrixResult<MatrixSyncBatch> {
    let next_batch =
        MatrixSyncToken::new(response.next_batch).map_err(|_| invalid_response(operation))?;
    let mut rooms = Vec::with_capacity(response.rooms.join.len());
    for (room_id, room) in response.rooms.join {
        let Ok(room_id) = MatrixRoomId::new(room_id) else {
            continue;
        };
        // 越界或缺字段的事件逐条丢掉，不让一条坏事件挡住整批。
        let timeline = room
            .timeline
            .events
            .iter()
            .filter_map(timeline_event)
            .collect();
        rooms.push(MatrixRoomSync::new(
            room_id,
            MatrixRoomSyncKind::Joined,
            room.timeline.limited,
            room.timeline
                .prev_batch
                .and_then(|token| MatrixBackfillToken::new(token).ok()),
            timeline,
            Vec::new(),
        ));
    }
    Ok(MatrixSyncBatch::new(next_batch, rooms))
}

fn timeline_event(raw: &Value) -> Option<MatrixTimelineEvent> {
    let event = raw.as_object()?;
    let text = |name: &str| event.get(name).and_then(Value::as_str);
    MatrixTimelineEvent::new(
        text("event_id").and_then(|value| MatrixEventId::new(value).ok()),
        text("sender").and_then(|value| MatrixUserId::new(value).ok()),
        MatrixEventType::new(text("type")?).ok()?,
        text("state_key").map(str::to_owned),
        event
            .get("unsigned")
            .and_then(|unsigned| unsigned.get("transaction_id"))
            .and_then(Value::as_str)
            .and_then(|value| MatrixTransactionId::new(value).ok()),
        event.get("origin_server_ts").and_then(Value::as_u64),
        event
            .get("content")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new())),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_room_application::ports::{
        MatrixFailureKind, MatrixSyncToken, NetworkAgentMatrixGateway as _,
        NetworkAgentSyncRequest, SecretValue,
    };
    use axum::{
        Json, Router,
        extract::{Query, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use serde_json::{Value, json};

    use super::MatrixAgentSessionClient;

    type QueryParams = std::collections::BTreeMap<String, String>;

    type ServerState = (Arc<Mutex<Seen>>, Arc<(StatusCode, Value)>);

    #[derive(Default)]
    struct Seen {
        authorization: Option<String>,
        query: QueryParams,
    }

    async fn serve(response: (StatusCode, Value)) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let state = (seen.clone(), Arc::new(response));
        let app = Router::new()
            .route("/_matrix/client/v3/sync", get(sync))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可以监听本机端口");
        let address = listener.local_addr().expect("有本机地址");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务器运行");
        });
        (format!("http://{address}"), seen)
    }

    async fn sync(
        State((seen, response)): State<ServerState>,
        headers: HeaderMap,
        Query(query): Query<QueryParams>,
    ) -> impl IntoResponse {
        let mut seen = seen.lock().unwrap();
        seen.authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        seen.query = query;
        (response.0, Json(response.1.clone()))
    }

    fn request(since: Option<&str>) -> NetworkAgentSyncRequest {
        NetworkAgentSyncRequest {
            since: since.map(|token| MatrixSyncToken::new(token).unwrap()),
            timeout_millis: 45_000,
            timeline_limit: 20,
        }
    }

    fn token() -> SecretValue {
        SecretValue::new("syt_network_agent").unwrap()
    }

    #[tokio::test]
    async fn 带着_agent_自己的令牌只同步消息事件_长轮询不超过三十秒() {
        let (url, seen) = serve((
            StatusCode::OK,
            json!({
                "next_batch": "s72595_4483_1934",
                "rooms": {"join": {"!lobby:matrix.test": {"timeline": {
                    "limited": true,
                    "prev_batch": "t392-516_47314_0_7_1_1_1_11444_1",
                    "events": [
                        {
                            "event_id": "$preview:matrix.test",
                            "sender": "@_agent_0198b60177a17bb883eba8fe68c97e44:matrix.test",
                            "type": "io.github.rainyflash.agentroom.message.preview.v1",
                            "origin_server_ts": 1_758_600_000_000_u64,
                            "unsigned": {"transaction_id": "agent-room-message-0198"},
                            "content": {"schemaVersion": "1.0"}
                        },
                        {"type": "io.github.rainyflash.agentroom.message.preview.v1"},
                        {"event_id": "$bad:matrix.test", "content": {}}
                    ]
                }}}}
            }),
        ))
        .await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();

        let batch = client
            .sync(&token(), &request(Some("s72594_4483_1934")))
            .await
            .expect("同步成功");

        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.authorization.as_deref(),
            Some("Bearer syt_network_agent")
        );
        assert_eq!(seen.query["since"], "s72594_4483_1934");
        assert_eq!(seen.query["timeout"], "30000");
        assert_eq!(seen.query["set_presence"], "offline");
        let filter: Value = serde_json::from_str(&seen.query["filter"]).unwrap();
        assert_eq!(filter["room"]["timeline"]["limit"], 20);
        assert_eq!(filter["room"]["state"]["types"], json!([]));
        assert_eq!(
            filter["room"]["timeline"]["types"][0],
            "io.github.rainyflash.agentroom.message.preview.v1"
        );

        assert_eq!(batch.next_batch().as_str(), "s72595_4483_1934");
        let room = &batch.rooms()[0];
        assert_eq!(room.room_id().as_str(), "!lobby:matrix.test");
        assert!(room.timeline_limited());
        assert!(room.previous_batch().is_some());
        // 没有类型的那条被丢掉；缺字段但类型齐全的保留，交给验签那一层隔离。
        assert_eq!(room.timeline().len(), 2);
        let event = &room.timeline()[0];
        assert_eq!(event.event_id().unwrap().as_str(), "$preview:matrix.test");
        assert_eq!(
            event.transaction_id().unwrap().as_str(),
            "agent-room-message-0198"
        );
        assert_eq!(event.origin_server_timestamp(), Some(1_758_600_000_000));
    }

    #[tokio::test]
    async fn 第一次同步不带位置() {
        let (url, seen) = serve((StatusCode::OK, json!({"next_batch": "s1"}))).await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();

        let batch = client
            .sync(&token(), &request(None))
            .await
            .expect("同步成功");

        assert!(!seen.lock().unwrap().query.contains_key("since"));
        assert!(batch.rooms().is_empty());
    }

    #[tokio::test]
    async fn 令牌失效映射为未认证() {
        let (url, _) = serve((
            StatusCode::UNAUTHORIZED,
            json!({"errcode": "M_UNKNOWN_TOKEN", "error": "Invalid access token"}),
        ))
        .await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();

        let failure = client
            .sync(&token(), &request(None))
            .await
            .expect_err("令牌失效");

        assert_eq!(failure.kind(), MatrixFailureKind::Unauthenticated);
    }

    /// 发言与离开的模拟服务器：记下方法、路径（未解码）与请求体，按预设回答。
    async fn serve_writes(
        response: (StatusCode, Value),
    ) -> (String, Arc<Mutex<Vec<(String, String, Value)>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let response = Arc::new(response);
        let app = Router::new().fallback(move |request: axum::extract::Request| {
            let recorded = recorded.clone();
            let response = response.clone();
            async move {
                let method = request.method().to_string();
                let path = request.uri().path().to_owned();
                let body = axum::body::to_bytes(request.into_body(), 64 * 1_024)
                    .await
                    .unwrap();
                let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                recorded.lock().unwrap().push((method, path, body));
                (response.0, Json(response.1.clone()))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可以监听本机端口");
        let address = listener.local_addr().expect("有本机地址");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务器运行");
        });
        (format!("http://{address}"), seen)
    }

    fn room() -> agent_room_application::ports::MatrixRoomId {
        agent_room_application::ports::MatrixRoomId::new("!lobby:matrix.test").unwrap()
    }

    #[tokio::test]
    async fn 以_agent_自己的身份发事件_路径逐段编码_事务_id_固定() {
        let (url, seen) =
            serve_writes((StatusCode::OK, json!({"event_id": "$sent:matrix.test"}))).await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();
        let event = agent_room_application::ports::MatrixEvent::new(
            agent_room_application::ports::MatrixEventType::new(
                "io.github.rainyflash.agentroom.message.preview.v1",
            )
            .unwrap(),
            agent_room_application::ports::MatrixTransactionId::new("agent-room-message-0198")
                .unwrap(),
            json!({"schemaVersion": "1.0"}),
        )
        .unwrap();

        let accepted = client
            .send_event(&token(), &room(), &event)
            .await
            .expect("发出去了");

        assert_eq!(accepted.event_id().as_str(), "$sent:matrix.test");
        assert_eq!(
            accepted.transaction_id().as_str(),
            "agent-room-message-0198"
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "PUT");
        assert_eq!(
            seen[0].1,
            "/_matrix/client/v3/rooms/!lobby:matrix.test/send/io.github.rainyflash.agentroom.message.preview.v1/agent-room-message-0198"
        );
        assert_eq!(seen[0].2, json!({"schemaVersion": "1.0"}));
    }

    #[tokio::test]
    async fn 在线状态写成房间状态_状态键逐段编码() {
        let (url, seen) =
            serve_writes((StatusCode::OK, json!({"event_id": "$status:matrix.test"}))).await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();
        let event = agent_room_application::ports::MatrixStateEvent::new(
            agent_room_application::ports::MatrixEventType::new(
                "io.github.rainyflash.agentroom.agent.status.v1",
            )
            .unwrap(),
            agent_room_application::ports::MatrixStateKey::new(
                "0198b601-77a1-7bb8-83eb-a8fe68c97e52",
            )
            .unwrap(),
            json!({"status": "idle"}),
        )
        .unwrap();

        let event_id = client
            .send_state_event(&token(), &room(), &event)
            .await
            .expect("写进去了");

        assert_eq!(event_id.as_str(), "$status:matrix.test");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "PUT");
        assert_eq!(
            seen[0].1,
            "/_matrix/client/v3/rooms/!lobby:matrix.test/state/io.github.rainyflash.agentroom.agent.status.v1/0198b601-77a1-7bb8-83eb-a8fe68c97e52"
        );
        assert_eq!(seen[0].2, json!({"status": "idle"}));
    }

    #[tokio::test]
    async fn 离开房间_已经不在里面也算成功() {
        let (url, seen) = serve_writes((StatusCode::OK, json!({}))).await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();
        client.leave(&token(), &room()).await.expect("离开了");
        assert_eq!(
            seen.lock().unwrap()[0].1,
            "/_matrix/client/v3/rooms/!lobby:matrix.test/leave"
        );

        let (url, _) = serve_writes((
            StatusCode::FORBIDDEN,
            json!({"errcode": "M_FORBIDDEN", "error": "User not in room"}),
        ))
        .await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();
        client
            .leave(&token(), &room())
            .await
            .expect("不在里面也算离开");

        let (url, _) = serve_writes((
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"errcode": "M_UNKNOWN", "error": "down"}),
        ))
        .await;
        let client =
            MatrixAgentSessionClient::new(&url, std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(
            client.leave(&token(), &room()).await.unwrap_err().kind(),
            MatrixFailureKind::DependencyUnavailable
        );
    }

    #[test]
    fn 生产地址必须用_https() {
        assert!(
            MatrixAgentSessionClient::new(
                "http://matrix.example.com",
                std::time::Duration::from_secs(2)
            )
            .is_err()
        );
        assert!(
            MatrixAgentSessionClient::new("http://synapse:8008", std::time::Duration::from_secs(2))
                .is_ok()
        );
    }
}
