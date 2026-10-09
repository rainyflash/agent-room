//! 不登录也能看公开大厅（specs/public-lobby-watch/design.md）：以建公开大厅的应用服务账号读大厅
//! 里最近的消息和当前状态。
//!
//! 请求不带 `user_id`，用的就是应用服务自己的账号：每个大厅分片都是它建的，它一直在里面，不冒充
//! 任何人。公开大厅不加密，读到的就是明文事件。写名片的 Agent 在不在线，也以它的身份问
//! （同在大厅里才问得到）。

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
    MatrixTimelineEvent, MatrixUserId, MatrixUserPresence, PortFuture, PublicLobbyMatrixReader,
};
use agent_room_domain::agent_lifecycle::MatrixPresenceState;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    MatrixApplicationServiceProvisioner,
    agent_sessions::{MESSAGE_EVENT_TYPES, timeline_event},
    decode_json, decode_matrix_error, map_matrix_error, map_transport_error, read_body_within,
    rooms::endpoint_with_segments,
};

/// 一次最多读这么多条消息事件。
const MAX_MESSAGES_LIMIT: u16 = 100;
/// 最近的消息一次最多读这么多字节：100 条消息事件正常远小于这个数。
const MAX_MESSAGES_RESPONSE_BYTES: usize = 4 * 1_024 * 1_024;
/// 房间状态最多读这么多字节。每个进过大厅的 Agent 实例都留着一条在线状态，日积月累，给足余量。
const MAX_STATE_RESPONSE_BYTES: usize = 16 * 1_024 * 1_024;
/// 一个人的在线状态只有几个字段。
const MAX_PRESENCE_RESPONSE_BYTES: usize = 16 * 1_024;

impl MatrixApplicationServiceProvisioner {
    async fn recent_messages_internal(
        &self,
        room_id: &MatrixRoomId,
        limit: u16,
    ) -> MatrixResult<Vec<MatrixTimelineEvent>> {
        let operation = MatrixOperation::Backfill;
        let mut url = endpoint_with_segments(
            &self.homeserver_url,
            &[
                "_matrix",
                "client",
                "v3",
                "rooms",
                room_id.as_str(),
                "messages",
            ],
            operation,
        )?;
        url.query_pairs_mut()
            .append_pair("dir", "b")
            .append_pair("limit", &limit.clamp(1, MAX_MESSAGES_LIMIT).to_string())
            .append_pair("filter", &json!({"types": MESSAGE_EVENT_TYPES}).to_string());
        let body = self
            .read_as_service(url, operation, MAX_MESSAGES_RESPONSE_BYTES)
            .await?;
        let page: MessagesPage = decode_json(&body, operation)?;
        // 不给起点就从最新往回翻，新的在前；倒过来按时间先后交出去。越界的事件逐条丢掉。
        let mut events: Vec<MatrixTimelineEvent> =
            page.chunk.iter().filter_map(timeline_event).collect();
        events.reverse();
        Ok(events)
    }

    async fn current_state_internal(
        &self,
        room_id: &MatrixRoomId,
    ) -> MatrixResult<Vec<MatrixTimelineEvent>> {
        let operation = MatrixOperation::ReadRoomState;
        let url = endpoint_with_segments(
            &self.homeserver_url,
            &[
                "_matrix",
                "client",
                "v3",
                "rooms",
                room_id.as_str(),
                "state",
            ],
            operation,
        )?;
        let body = self
            .read_as_service(url, operation, MAX_STATE_RESPONSE_BYTES)
            .await?;
        let state: Vec<Value> = decode_json(&body, operation)?;
        Ok(state.iter().filter_map(timeline_event).collect())
    }

    async fn user_presence_internal(
        &self,
        user_id: &MatrixUserId,
    ) -> MatrixResult<MatrixUserPresence> {
        let operation = MatrixOperation::ReadPresence;
        let url = endpoint_with_segments(
            &self.homeserver_url,
            &[
                "_matrix",
                "client",
                "v3",
                "presence",
                user_id.as_str(),
                "status",
            ],
            operation,
        )?;
        let body = self
            .read_as_service(url, operation, MAX_PRESENCE_RESPONSE_BYTES)
            .await?;
        let answer: PresenceAnswer = decode_json(&body, operation)?;
        let state = MatrixPresenceState::from_matrix(&answer.presence)
            .ok_or_else(|| MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse))?;
        Ok(MatrixUserPresence::new(
            user_id.clone(),
            state,
            answer.last_active_ago,
        ))
    }

    /// 以应用服务自己的账号发一个 GET，成功时交回不超过 `limit` 字节的正文。
    async fn read_as_service(
        &self,
        url: Url,
        operation: MatrixOperation,
        limit: usize,
    ) -> MatrixResult<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .bearer_auth(self.access_token.expose())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_body_within(response, operation, limit).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        Ok(body)
    }
}

impl PublicLobbyMatrixReader for MatrixApplicationServiceProvisioner {
    fn recent_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        limit: u16,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>> {
        Box::pin(self.recent_messages_internal(room_id, limit))
    }

    fn current_state<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Vec<MatrixTimelineEvent>>> {
        Box::pin(self.current_state_internal(room_id))
    }

    fn user_presence<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixUserPresence>> {
        Box::pin(self.user_presence_internal(user_id))
    }
}

/// `/rooms/{roomId}/messages` 的回答里只用得到 `chunk`。
#[derive(Deserialize)]
struct MessagesPage {
    #[serde(default)]
    chunk: Vec<Value>,
}

/// `/presence/{userId}/status` 的回答里只用得到这两样。
#[derive(Deserialize)]
struct PresenceAnswer {
    presence: String,
    #[serde(default)]
    last_active_ago: Option<u64>,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use agent_room_application::ports::{
        MatrixFailureKind, MatrixOperation, MatrixRoomId, MatrixUserId,
        PublicLobbyMatrixReader as _, SecretValue,
    };
    use agent_room_domain::agent_lifecycle::MatrixPresenceState;
    use axum::{
        Json, Router,
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use serde_json::{Value, json};

    use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

    type QueryParams = BTreeMap<String, String>;

    /// 记下每个请求的路径、查询参数和认证头，按路径给出预设的回答。
    #[derive(Default)]
    struct Seen {
        requests: Vec<(String, QueryParams, Option<String>)>,
    }

    type ServerState = (Arc<Mutex<Seen>>, Arc<(StatusCode, Value, Value)>);

    async fn serve(
        status: StatusCode,
        messages: Value,
        state: Value,
    ) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let app = Router::new()
            .route(
                "/_matrix/client/v3/rooms/{room_id}/{endpoint}",
                get(respond),
            )
            .with_state((seen.clone(), Arc::new((status, messages, state))));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可以监听本机端口");
        let address = listener.local_addr().expect("有本机地址");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务器运行");
        });
        (format!("http://{address}"), seen)
    }

    async fn respond(
        State((seen, answers)): State<ServerState>,
        Path((room_id, endpoint)): Path<(String, String)>,
        headers: HeaderMap,
        Query(query): Query<QueryParams>,
    ) -> impl IntoResponse {
        seen.lock().unwrap().requests.push((
            format!("{room_id}/{endpoint}"),
            query,
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        ));
        let body = if endpoint == "messages" {
            answers.1.clone()
        } else {
            answers.2.clone()
        };
        (answers.0, Json(body))
    }

    fn reader(url: &str) -> MatrixApplicationServiceProvisioner {
        MatrixApplicationServiceProvisioner::new(
            MatrixApplicationServiceConfiguration::new(
                url,
                "matrix.test",
                SecretValue::new("application-service-secret").unwrap(),
                Duration::from_secs(2),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn room() -> MatrixRoomId {
        MatrixRoomId::new("!lobby:matrix.test").unwrap()
    }

    #[tokio::test]
    async fn 以应用服务自己的账号从最新往回读消息事件_按时间先后交出() {
        let (url, seen) = serve(
            StatusCode::OK,
            json!({
                "start": "t9",
                "end": "t7",
                "chunk": [
                    {
                        "event_id": "$newer:matrix.test",
                        "sender": "@ada:matrix.test",
                        "type": "io.github.rainyflash.agentroom.message.preview.v2",
                        "origin_server_ts": 1_758_600_000_002_u64,
                        "content": {"schemaVersion": "2.0"}
                    },
                    {
                        "event_id": "$older:matrix.test",
                        "sender": "@ada:matrix.test",
                        "type": "io.github.rainyflash.agentroom.message.preview.v2",
                        "origin_server_ts": 1_758_600_000_001_u64,
                        "content": {"schemaVersion": "2.0"}
                    },
                    {"event_id": "$untyped:matrix.test", "content": {}}
                ]
            }),
            json!([]),
        )
        .await;

        let events = reader(&url)
            .recent_messages(&room(), 60)
            .await
            .expect("读到最近的消息");

        let ids: Vec<&str> = events
            .iter()
            .map(|event| event.event_id().unwrap().as_str())
            .collect();
        assert_eq!(ids, ["$older:matrix.test", "$newer:matrix.test"]);
        let seen = seen.lock().unwrap();
        let (path, query, authorization) = &seen.requests[0];
        assert_eq!(path, "!lobby:matrix.test/messages");
        assert_eq!(
            authorization.as_deref(),
            Some("Bearer application-service-secret")
        );
        // 不冒充任何用户，也不给起点。
        assert!(!query.contains_key("user_id"));
        assert!(!query.contains_key("from"));
        assert_eq!(query["dir"], "b");
        assert_eq!(query["limit"], "60");
        let filter: Value = serde_json::from_str(&query["filter"]).unwrap();
        assert_eq!(
            filter["types"],
            json!([
                "io.github.rainyflash.agentroom.message.preview.v1",
                "io.github.rainyflash.agentroom.message.revision.v1",
                "io.github.rainyflash.agentroom.message.preview.v2",
                "io.github.rainyflash.agentroom.message.revision.v2",
            ])
        );
    }

    #[tokio::test]
    async fn 读房间当前状态_带着状态键() {
        let (url, seen) = serve(
            StatusCode::OK,
            json!({"chunk": []}),
            json!([
                {
                    "event_id": "$create:matrix.test",
                    "sender": "@_agent_room:matrix.test",
                    "type": "m.room.create",
                    "state_key": "",
                    "origin_server_ts": 1_u64,
                    "content": {"room_version": "11"}
                },
                {
                    "event_id": "$status:matrix.test",
                    "sender": "@_agent_x:matrix.test",
                    "type": "io.github.rainyflash.agentroom.agent.status.v1",
                    "state_key": "0198b601-77a1-7bb8-83eb-a8fe68c97e45",
                    "origin_server_ts": 2_u64,
                    "content": {"status": "idle"}
                }
            ]),
        )
        .await;

        let state = reader(&url)
            .current_state(&room())
            .await
            .expect("读到房间状态");

        assert_eq!(state.len(), 2);
        assert_eq!(state[0].event_type().as_str(), "m.room.create");
        assert_eq!(state[0].state_key(), Some(""));
        assert_eq!(
            state[1].state_key(),
            Some("0198b601-77a1-7bb8-83eb-a8fe68c97e45")
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.requests[0].0, "!lobby:matrix.test/state");
        assert!(!seen.requests[0].1.contains_key("user_id"));
    }

    /// 按用户给出预设的在线状态，记下问的是谁、带的什么认证头。
    async fn serve_presence(
        answers: Vec<(&'static str, StatusCode, Value)>,
    ) -> (String, Arc<Mutex<Seen>>) {
        type PresenceState = (
            Arc<Mutex<Seen>>,
            Arc<Vec<(&'static str, StatusCode, Value)>>,
        );
        async fn respond(
            State((seen, answers)): State<PresenceState>,
            Path(user_id): Path<String>,
            headers: HeaderMap,
        ) -> impl IntoResponse {
            seen.lock().unwrap().requests.push((
                user_id.clone(),
                QueryParams::new(),
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            ));
            let (_, status, body) = answers
                .iter()
                .find(|(user, _, _)| *user == user_id)
                .cloned()
                .unwrap_or(("", StatusCode::FORBIDDEN, json!({"errcode": "M_FORBIDDEN"})));
            (status, Json(body))
        }
        let seen = Arc::new(Mutex::new(Seen::default()));
        let app = Router::new()
            .route("/_matrix/client/v3/presence/{user_id}/status", get(respond))
            .with_state((seen.clone(), Arc::new(answers)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可以监听本机端口");
        let address = listener.local_addr().expect("有本机地址");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务器运行");
        });
        (format!("http://{address}"), seen)
    }

    #[tokio::test]
    async fn 以应用服务自己的账号问大厅里的人的在线状态() {
        let (url, seen) = serve_presence(vec![
            (
                "@_agent_x:matrix.test",
                StatusCode::OK,
                json!({"presence": "unavailable", "last_active_ago": 42_000, "currently_active": false}),
            ),
            (
                "@_agent_y:matrix.test",
                StatusCode::OK,
                json!({"presence": "busy"}),
            ),
        ])
        .await;
        let reader = reader(&url);

        let presence = reader
            .user_presence(&MatrixUserId::new("@_agent_x:matrix.test").unwrap())
            .await
            .expect("问得到在线状态");
        assert_eq!(presence.state(), MatrixPresenceState::Unavailable);
        assert_eq!(presence.last_active_ago_ms(), Some(42_000));
        assert_eq!(presence.user_id().as_str(), "@_agent_x:matrix.test");

        let unknown = reader
            .user_presence(&MatrixUserId::new("@_agent_y:matrix.test").unwrap())
            .await
            .unwrap_err();
        assert_eq!(unknown.kind(), MatrixFailureKind::InvalidResponse);
        let stranger = reader
            .user_presence(&MatrixUserId::new("@stranger:matrix.test").unwrap())
            .await
            .unwrap_err();
        assert_eq!(stranger.kind(), MatrixFailureKind::Forbidden);
        assert_eq!(stranger.operation(), MatrixOperation::ReadPresence);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.requests[0].0, "@_agent_x:matrix.test");
        assert_eq!(
            seen.requests[0].2.as_deref(),
            Some("Bearer application-service-secret")
        );
    }

    #[tokio::test]
    async fn 读不了时照实报错_不当成空房间() {
        let (url, _) = serve(
            StatusCode::FORBIDDEN,
            json!({"errcode": "M_FORBIDDEN"}),
            json!({"errcode": "M_FORBIDDEN"}),
        )
        .await;
        let reader = reader(&url);

        let messages = reader.recent_messages(&room(), 60).await.unwrap_err();
        let state = reader.current_state(&room()).await.unwrap_err();

        assert_eq!(messages.kind(), MatrixFailureKind::Forbidden);
        assert_eq!(messages.operation(), MatrixOperation::Backfill);
        assert_eq!(state.kind(), MatrixFailureKind::Forbidden);
        assert_eq!(state.operation(), MatrixOperation::ReadRoomState);
    }
}
