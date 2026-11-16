use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agent_room_application::ports::{
    MatrixFailureKind, MatrixRoomId, MatrixUserId, ReminderMatrixReader, SecretValue,
};
use agent_room_domain::time::UtcMillis;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex};

use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

const AS_TOKEN: &str = "as-token-for-tests";
const SERVER: &str = "matrix.agent-room.localhost";
const READER: &str = "@reader:matrix.agent-room.localhost";
const OTHER: &str = "@other:matrix.agent-room.localhost";
const ROOM: &str = "!room:matrix.agent-room.localhost";
const LOCKED: &str = "!locked:matrix.agent-room.localhost";
const WORKSPACE: &str = "io.github.rainyflash.agentroom.personal-workspace.v1";

/// 假的 Synapse：他的个人工作区，房间里从新到旧的事件；记下每个请求的查询参数。
struct Synapse {
    workspace: Option<Value>,
    events: Vec<Value>,
    queries: Vec<BTreeMap<String, String>>,
}

type Shared = Arc<Mutex<Synapse>>;

fn synapse(workspace: Option<Value>, events: Vec<Value>) -> Shared {
    Arc::new(Mutex::new(Synapse {
        workspace,
        events,
        queries: Vec::new(),
    }))
}

fn event(id: &str, sender: &str, at: i64, mentions: &[&str]) -> Value {
    json!({
        "event_id": id,
        "sender": sender,
        "origin_server_ts": at,
        "type": "io.github.rainyflash.agentroom.message.preview.v1",
        "content": { "preview": { "conversation": { "mentions": mentions } } },
    })
}

fn not_authorized(headers: &HeaderMap) -> Option<Response> {
    let expected = format!("Bearer {AS_TOKEN}");
    (headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str()))
    .then(|| StatusCode::UNAUTHORIZED.into_response())
}

async fn account_data(
    State(state): State<Shared>,
    Path((user, kind)): Path<(String, String)>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(refused) = not_authorized(&headers) {
        return refused;
    }
    assert_eq!(user, READER);
    assert_eq!(kind, WORKSPACE);
    let mut synapse = state.lock().await;
    synapse.queries.push(query);
    match &synapse.workspace {
        Some(document) => Json(document.clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "errcode": "M_NOT_FOUND", "error": "Account data not found" })),
        )
            .into_response(),
    }
}

async fn messages(
    State(state): State<Shared>,
    Path(room): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(refused) = not_authorized(&headers) {
        return refused;
    }
    if room == LOCKED {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "errcode": "M_FORBIDDEN", "error": "not in room" })),
        )
            .into_response();
    }
    let mut synapse = state.lock().await;
    let from: usize = query.get("from").map_or(0, |from| from.parse().unwrap());
    let limit: usize = query["limit"].parse().unwrap();
    synapse.queries.push(query);
    let chunk: Vec<Value> = synapse
        .events
        .iter()
        .skip(from)
        .take(limit)
        .cloned()
        .collect();
    let next = from + chunk.len();
    let end = (next < synapse.events.len()).then(|| next.to_string());
    Json(json!({ "chunk": chunk, "start": from.to_string(), "end": end })).into_response()
}

async fn start(state: Shared) -> String {
    let app = Router::new()
        .route(
            "/_matrix/client/v3/user/{user}/account_data/{kind}",
            get(account_data),
        )
        .route("/_matrix/client/v3/rooms/{room}/messages", get(messages))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试端口可用");
    let address = listener.local_addr().expect("测试地址有效");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("测试服务可运行");
    });
    format!("http://{address}")
}

fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
    MatrixApplicationServiceProvisioner::new(
        MatrixApplicationServiceConfiguration::new(
            url,
            SERVER.to_owned(),
            SecretValue::new(AS_TOKEN.to_owned()).expect("令牌有效"),
            Duration::from_secs(5),
        )
        .expect("配置有效"),
    )
    .expect("客户端有效")
}

fn reader() -> MatrixUserId {
    MatrixUserId::new(READER).unwrap()
}

fn room(id: &str) -> MatrixRoomId {
    MatrixRoomId::new(id).unwrap()
}

fn at(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

#[tokio::test]
async fn 个人工作区替本人读_没写过是空的() {
    let document = json!({ "schema": 1, "entries": [] });
    let state = synapse(Some(document.clone()), Vec::new());
    let url = start(state.clone()).await;

    let read = provisioner(&url)
        .personal_workspace(&reader())
        .await
        .expect("读得到");
    assert_eq!(read, Some(document));
    assert_eq!(
        state.lock().await.queries[0]
            .get("user_id")
            .map(String::as_str),
        Some(READER),
        "冒充本人"
    );

    state.lock().await.workspace = None;
    assert_eq!(
        provisioner(&url).personal_workspace(&reader()).await,
        Ok(None)
    );
}

#[tokio::test]
async fn 从新往旧翻_翻到比起点早的就停_不加密的看有没有点他() {
    let mut events: Vec<Value> = (0..150)
        .map(|index| event(&format!("$new{index}"), OTHER, 10_000 - index, &[]))
        .collect();
    events[3] = event("$mention", OTHER, 9_997, &[READER]);
    events.push(event("$old", OTHER, 1_000, &[READER]));
    let state = synapse(None, events);
    let url = start(state.clone()).await;

    let page = provisioner(&url)
        .messages_since(&reader(), &room(ROOM), false, at(9_851))
        .await
        .expect("读得到");

    assert_eq!(page.messages.len(), 150);
    assert!(!page.truncated);
    assert_eq!(
        page.messages
            .iter()
            .filter(|message| message.mentions_reader)
            .map(|message| message.event_id.as_str())
            .collect::<Vec<_>>(),
        ["$mention"]
    );
    let queries = state.lock().await.queries.clone();
    assert_eq!(queries.len(), 2, "翻了两页");
    assert_eq!(queries[1].get("from").map(String::as_str), Some("100"));
    let filter: Value = serde_json::from_str(&queries[0]["filter"]).unwrap();
    assert_eq!(
        filter["types"],
        json!([
            "io.github.rainyflash.agentroom.message.preview.v1",
            "io.github.rainyflash.agentroom.message.preview.v2"
        ])
    );
    assert!(queries.iter().all(|query| query["user_id"] == READER));
}

#[tokio::test]
async fn 加密的房间只看外面那一层_翻满两页还没到起点就说还有() {
    let events: Vec<Value> = (0..250)
        .map(|index| event(&format!("$e{index}"), OTHER, 10_000 - index, &[READER]))
        .collect();
    let state = synapse(None, events);
    let url = start(state.clone()).await;

    let page = provisioner(&url)
        .messages_since(&reader(), &room(ROOM), true, at(1))
        .await
        .expect("读得到");

    assert_eq!(page.messages.len(), 200);
    assert!(page.truncated);
    assert!(page.messages.iter().all(|message| !message.mentions_reader));
    let filter: Value = serde_json::from_str(&state.lock().await.queries[0]["filter"]).unwrap();
    assert_eq!(filter["types"], json!(["m.room.encrypted"]));
}

#[tokio::test]
async fn 读不了的房间照实报() {
    let url = start(synapse(None, Vec::new())).await;
    let failure = provisioner(&url)
        .messages_since(&reader(), &room(LOCKED), true, at(1))
        .await
        .expect_err("读不了");
    assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
}
