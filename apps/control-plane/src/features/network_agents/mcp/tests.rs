use std::sync::Arc;

use agent_room_application::network_agents::{NetworkAgentFailure, NetworkAgentFailureKind};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::tests::{FakeAgents, FakeMessaging, TOKEN, app_with};
use crate::network_gateway::NetworkGatewayFailure;

const NOW: i64 = 1_758_600_000_000;

fn app(agents: Arc<FakeAgents>, messaging: Arc<FakeMessaging>) -> axum::Router {
    app_with(agents, messaging, NOW)
}

fn post(body: &Value, bearer: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        // Caddy 转发时总会带上原来的 Host；rmcp 要先解析它。
        .header(header::HOST, "api.agent-room.example")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-06-18")
        .header("x-forwarded-for", "198.51.100.7");
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    request.body(Body::from(body.to_string())).unwrap()
}

fn call(name: &str, arguments: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
           "params": {"name": name, "arguments": arguments}})
}

/// 取出 JSON-RPC 响应：流式响应里找带 id 的那一条 `data:`，否则整个响应体就是 JSON。
async fn rpc(app: axum::Router, body: &Value, bearer: Option<&str>) -> Value {
    let response = app.oneshot(post(body, bearer)).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|value| value.get("id").is_some())
        .or_else(|| serde_json::from_str(text).ok())
        .unwrap_or_else(|| panic!("没有 JSON-RPC 响应：{text}"))
}

#[tokio::test]
async fn 协商后列出七个工具_说明里写明令牌用法与安全边界() {
    let app = app(
        Arc::new(FakeAgents::default()),
        Arc::new(FakeMessaging::default()),
    );

    let init = rpc(
        app.clone(),
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"}}}),
        None,
    )
    .await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("agent_room_join") && instructions.contains("token"));
    assert!(instructions.contains("不可信"));

    let list = rpc(
        app,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        None,
    )
    .await;
    let mut names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "agent_room_ack",
            "agent_room_get_self",
            "agent_room_join",
            "agent_room_leave",
            "agent_room_list_rooms",
            "agent_room_send_message",
            "agent_room_wait_for_messages",
        ]
    );
}

#[tokio::test]
async fn 起名进大厅返回令牌_来源按转发地址算() {
    let agents = Arc::new(FakeAgents::default());

    let joined = rpc(
        app(agents.clone(), Arc::new(FakeMessaging::default())),
        &call("agent_room_join", &json!({"name": "Scout"})),
        None,
    )
    .await;

    let result = &joined["result"];
    assert_ne!(result["isError"], true, "{joined}");
    assert_eq!(result["structuredContent"]["token"], TOKEN);
    assert_eq!(result["structuredContent"]["displayName"], "Scout 2");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("保存 token")
    );
    let created = agents.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].name, "Scout");
    assert_eq!(created[0].room, None);
    assert_ne!(created[0].source_digest, [0; 32]);
}

#[tokio::test]
async fn 请求头里的令牌优先_没有时用参数_都没有就是未认证() {
    let agents = Arc::new(FakeAgents::default());
    let messaging = Arc::new(FakeMessaging::default());
    let app = app(agents.clone(), messaging.clone());

    let waited = rpc(
        app.clone(),
        &call(
            "agent_room_wait_for_messages",
            &json!({"token": "ignored-when-header-present", "waitSeconds": 5}),
        ),
        Some(TOKEN),
    )
    .await;
    assert_ne!(waited["result"]["isError"], true, "{waited}");
    assert_eq!(waited["result"]["structuredContent"]["pending"], 3);
    assert!(
        waited["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("不可信")
    );
    {
        let waits = messaging.waits.lock().unwrap();
        assert_eq!(waits[0].0, TOKEN);
        assert_eq!(waits[0].1, std::time::Duration::from_secs(5));
        assert_eq!(waits[0].2, 20);
    }

    let sent = rpc(
        app.clone(),
        &call(
            "agent_room_send_message",
            &json!({"token": TOKEN, "text": "大家好", "mentions": ["@ada:matrix.test"]}),
        ),
        None,
    )
    .await;
    assert_eq!(
        sent["result"]["structuredContent"]["status"], "sent",
        "{sent}"
    );
    {
        let drafts = messaging.drafts.lock().unwrap();
        assert_eq!(drafts[0].0, TOKEN);
        assert_eq!(drafts[0].1.text, "大家好");
        assert_eq!(drafts[0].1.mentions, ["@ada:matrix.test"]);
    }

    let anonymous = rpc(app, &call("agent_room_get_self", &json!({})), None).await;
    assert_eq!(anonymous["result"]["isError"], true);
    assert_eq!(
        anonymous["result"]["structuredContent"]["code"],
        "network_agent.unauthorized"
    );
    assert!(
        anonymous["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("[network_agent.unauthorized]")
    );
    assert_eq!(agents.tokens(), [""]);
}

#[tokio::test]
async fn 网关与用例的失败都用同样的错误码回答() {
    let messaging = FakeMessaging::failing(NetworkGatewayFailure::RoomRequired);
    let app_with_room_failure = app(Arc::new(FakeAgents::default()), messaging);
    let sent = rpc(
        app_with_room_failure,
        &call("agent_room_send_message", &json!({"text": "hi"})),
        Some(TOKEN),
    )
    .await;
    assert_eq!(sent["result"]["isError"], true);
    assert_eq!(
        sent["result"]["structuredContent"]["code"],
        "network_agent.room_required"
    );

    let disabled = FakeAgents::failing(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
    let rooms = rpc(
        app(disabled, Arc::new(FakeMessaging::default())),
        &call("agent_room_list_rooms", &json!({})),
        None,
    )
    .await;
    assert_eq!(
        rooms["result"]["structuredContent"]["code"],
        "network_agent.disabled"
    );
}

#[tokio::test]
async fn 列出大厅不要令牌_确认与离开交给网关() {
    let messaging = Arc::new(FakeMessaging::default());
    let app = app(Arc::new(FakeAgents::default()), messaging.clone());

    let rooms = rpc(
        app.clone(),
        &call("agent_room_list_rooms", &json!({})),
        None,
    )
    .await;
    assert_eq!(
        rooms["result"]["structuredContent"]["rooms"][0]["default"],
        true
    );

    let acked = rpc(
        app.clone(),
        &call("agent_room_ack", &json!({"eventId": "$hello:matrix.test"})),
        Some(TOKEN),
    )
    .await;
    assert_eq!(acked["result"]["structuredContent"]["acknowledged"], true);

    let left = rpc(app, &call("agent_room_leave", &json!({})), Some(TOKEN)).await;
    assert_eq!(left["result"]["structuredContent"]["left"], true);
    assert_eq!(*messaging.disabled.lock().unwrap(), [TOKEN]);
    assert_eq!(messaging.acks.lock().unwrap()[0].1, "$hello:matrix.test");
}
