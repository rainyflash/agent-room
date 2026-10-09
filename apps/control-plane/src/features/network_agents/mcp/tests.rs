use std::{sync::Arc, time::Duration};

use agent_room_application::network_agents::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentRoomRequest,
};
use agent_room_bridge_ipc::{IpcTimelineGap, wake::WakeRule};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::tests::{FakeAgents, FakeMessaging, PRIVATE_CATALOG_UUID, TOKEN, app_with};
use crate::network_gateway::{NetworkAgentRoomQuery, NetworkGatewayFailure};

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

fn initialize() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "test", "version": "1"}}})
}

fn list_tools() -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
}

/// 收集 JSON 里带中文的字符串，连同它在哪儿。
fn chinese_strings(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::String(text) if text.chars().any(is_chinese) => {
            found.push(format!("{path}: {text}"));
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                chinese_strings(item, &format!("{path}[{index}]"), found);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                chinese_strings(item, &format!("{path}.{key}"), found);
            }
        }
        _ => {}
    }
}

fn is_chinese(character: char) -> bool {
    matches!(
        character,
        '\u{3000}'..='\u{303f}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'
    )
}

#[tokio::test]
async fn 目录和宿主读到的服务说明_工具与参数说明都是英文() {
    // MCP 目录把这些原样展示给用户，用户多半读英文；参数说明来自输入结构的文档注释。
    let app = app(
        Arc::new(FakeAgents::default()),
        Arc::new(FakeMessaging::default()),
    );

    let init = rpc(app.clone(), &initialize(), None).await;
    let list = rpc(app, &list_tools(), None).await;

    let mut found = Vec::new();
    chinese_strings(&init["result"], "initialize", &mut found);
    chinese_strings(&list["result"], "tools/list", &mut found);
    assert!(found.is_empty(), "还有中文：\n{}", found.join("\n"));
}

#[tokio::test]
async fn 协商后列出十个工具_说明里写明令牌用法_口令与安全边界() {
    let app = app(
        Arc::new(FakeAgents::default()),
        Arc::new(FakeMessaging::default()),
    );

    let init = rpc(app.clone(), &initialize(), None).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    // 有的 MCP 宿主只读服务说明的前 1536 字节，再长后面的安全边界就被截掉了。
    assert!(
        instructions.len() <= 1536,
        "服务说明 {} 字节，超过 1536",
        instructions.len()
    );
    assert!(instructions.contains("agent_room_join") && instructions.contains("token"));
    assert!(instructions.contains("code") && instructions.contains("agent_room_enter_room"));
    assert!(instructions.contains("room number") && instructions.contains("knock"));
    assert!(instructions.contains("untrusted"));
    assert!(instructions.contains("agent_room_room_messages"));
    assert!(instructions.contains("agent_room_get_messages"));

    let list = rpc(app, &list_tools(), None).await;
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
            "agent_room_enter_room",
            "agent_room_get_messages",
            "agent_room_get_self",
            "agent_room_join",
            "agent_room_leave",
            "agent_room_list_rooms",
            "agent_room_room_messages",
            "agent_room_send_message",
            "agent_room_wait_for_messages",
        ]
    );
    let viewing = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tool| {
            matches!(
                tool["name"].as_str(),
                Some("agent_room_get_messages" | "agent_room_room_messages")
            )
        });
    for tool in viewing {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
        assert_eq!(tool["annotations"]["idempotentHint"], true, "{tool}");
    }
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
            .contains("Save the token")
    );
    let created = agents.created();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].name, "Scout");
    assert_eq!(created[0].room, NetworkAgentRoomRequest::Lobby(None));
    assert_ne!(created[0].source_digest, [0; 32]);
}

#[tokio::test]
async fn 凭口令起名_再进一个房间_大厅与口令只能给一个() {
    let agents = Arc::new(FakeAgents::default());
    let messaging = Arc::new(FakeMessaging::default());

    let joined = rpc(
        app(agents.clone(), messaging.clone()),
        &call(
            "agent_room_join",
            &json!({"name": "Scout", "code": "K7P3-Q9XW-2DMA"}),
        ),
        None,
    )
    .await;
    assert_ne!(joined["result"]["isError"], true, "{joined}");
    assert_eq!(
        agents.created()[0].room,
        NetworkAgentRoomRequest::Code("K7P3-Q9XW-2DMA".to_owned())
    );

    let entered = rpc(
        app(agents.clone(), messaging.clone()),
        &call(
            "agent_room_enter_room",
            &json!({"token": TOKEN, "room": "general"}),
        ),
        None,
    )
    .await;
    assert_ne!(entered["result"]["isError"], true, "{entered}");
    assert_eq!(
        entered["result"]["structuredContent"]["room"]["matrixRoomId"],
        "!lobby:matrix.test"
    );
    let calls = messaging.entered.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, TOKEN);
    assert_eq!(
        calls[0].1,
        NetworkAgentRoomRequest::Lobby(Some("general".to_owned()))
    );
    assert_ne!(calls[0].2, [0; 32]);

    for (tool, arguments) in [
        (
            "agent_room_join",
            json!({"name": "Scout", "room": "general", "code": "K7P3-Q9XW-2DMA"}),
        ),
        (
            "agent_room_enter_room",
            json!({"token": TOKEN, "room": "general", "code": "K7P3-Q9XW-2DMA"}),
        ),
    ] {
        let failed = rpc(
            app(agents.clone(), messaging.clone()),
            &call(tool, &arguments),
            None,
        )
        .await;
        assert_eq!(failed["result"]["isError"], true, "{tool}");
        assert_eq!(
            failed["result"]["structuredContent"]["code"], "network_agent.invalid_request",
            "{tool}"
        );
    }
    assert_eq!(agents.created().len(), 1, "写错的不交给网关");
    assert_eq!(messaging.entered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 拿房间号起名和再进一个房间都是敲门_第一段话说等管理者放行() {
    let agents = Arc::new(FakeAgents::default());
    let messaging = Arc::new(FakeMessaging::default());

    let joined = rpc(
        app(agents.clone(), messaging.clone()),
        &call(
            "agent_room_join",
            &json!({"name": "Scout", "room": PRIVATE_CATALOG_UUID}),
        ),
        None,
    )
    .await;
    let result = &joined["result"];
    assert_ne!(result["isError"], true, "{joined}");
    assert_eq!(result["structuredContent"]["token"], TOKEN);
    assert_eq!(result["structuredContent"]["knock"]["status"], "waiting");
    assert!(result["structuredContent"].get("room").is_none());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Knocked") && text.contains("Save the token"),
        "{text}"
    );

    let address = format!("https://agentroom.chat/lobby/{PRIVATE_CATALOG_UUID}");
    let entered = rpc(
        app(agents.clone(), messaging.clone()),
        &call(
            "agent_room_enter_room",
            &json!({"token": TOKEN, "room": address}),
        ),
        None,
    )
    .await;
    let result = &entered["result"];
    assert_ne!(result["isError"], true, "{entered}");
    assert_eq!(
        result["structuredContent"]["knock"]["catalogId"],
        PRIVATE_CATALOG_UUID
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Knocked")
    );
    assert_eq!(
        messaging.entered.lock().unwrap()[0].1,
        NetworkAgentRoomRequest::Lobby(Some(address)),
        "房间网址原样交给网关"
    );

    *messaging.declined.lock().unwrap() = true;
    let declined = rpc(
        app(agents, messaging),
        &call(
            "agent_room_enter_room",
            &json!({"token": TOKEN, "room": PRIVATE_CATALOG_UUID}),
        ),
        None,
    )
    .await;
    assert_eq!(
        declined["result"]["structuredContent"]["knock"]["status"],
        "declined"
    );
    assert!(
        declined["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("didn't let you in")
    );
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
            .contains("untrusted")
    );
    {
        let waits = messaging.waits.lock().unwrap();
        assert_eq!(waits[0].0, TOKEN);
        assert_eq!(waits[0].1.wait, Duration::from_secs(5));
        assert_eq!(waits[0].1.limit, 20);
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
    let room_acked = rpc(
        app.clone(),
        &call(
            "agent_room_ack",
            &json!({"eventId": "$hello:matrix.test", "roomId": "!lobby:matrix.test"}),
        ),
        Some(TOKEN),
    )
    .await;
    assert_eq!(room_acked["result"]["structuredContent"]["pending"], 2);

    let left = rpc(app, &call("agent_room_leave", &json!({})), Some(TOKEN)).await;
    assert_eq!(left["result"]["structuredContent"]["left"], true);
    assert_eq!(*messaging.disabled.lock().unwrap(), [TOKEN]);
    let acks = messaging.acks.lock().unwrap();
    assert_eq!(acks[0].1, "$hello:matrix.test");
    assert_eq!(acks[0].2, None);
    assert_eq!(acks[1].2.as_deref(), Some("!lobby:matrix.test"));
}

#[tokio::test]
async fn 等消息的叫醒规则和等谁也能用参数指定_写错了指出是哪一项() {
    let messaging = Arc::new(FakeMessaging::default());
    let app = app(Arc::new(FakeAgents::default()), messaging.clone());

    let waited = rpc(
        app.clone(),
        &call(
            "agent_room_wait_for_messages",
            &json!({"wake": "all", "waitFor": ["mentioned"], "settleSeconds": 0, "digestMinutes": 60}),
        ),
        Some(TOKEN),
    )
    .await;
    assert_ne!(waited["result"]["isError"], true, "{waited}");
    let content = &waited["result"]["structuredContent"];
    assert_eq!(content["wake"]["reason"], "messages");
    assert_eq!(content["skipped"], 2);
    assert_eq!(content["remaining"], 4);
    {
        let waits = messaging.waits.lock().unwrap();
        let request = &waits[0].1;
        assert!(request.wait_for_mentioned);
        assert_eq!(request.options.wake, WakeRule::All);
        assert_eq!(request.options.settle, Duration::ZERO);
        assert_eq!(request.options.digest, Some(Duration::from_hours(1)));
    }

    let scoped = rpc(
        app.clone(),
        &call(
            "agent_room_wait_for_messages",
            &json!({"roomId": "!lobby:matrix.test", "mentionsOnly": true}),
        ),
        Some(TOKEN),
    )
    .await;
    assert_ne!(scoped["result"]["isError"], true, "{scoped}");
    {
        let waits = messaging.waits.lock().unwrap();
        assert_eq!(waits[1].1.room.as_deref(), Some("!lobby:matrix.test"));
        assert!(waits[1].1.options.mentions_only);
    }

    for (arguments, field) in [
        (json!({"settleSeconds": 31}), "settle"),
        (json!({"mentionsOnly": true, "wake": "all"}), "mentionsOnly"),
    ] {
        let failed = rpc(
            app.clone(),
            &call("agent_room_wait_for_messages", &arguments),
            Some(TOKEN),
        )
        .await;
        assert_eq!(failed["result"]["isError"], true, "{failed}");
        let error = &failed["result"]["structuredContent"];
        assert_eq!(error["code"], "network_agent.invalid_request");
        assert_eq!(error["details"]["field"], field);
    }
    assert_eq!(messaging.waits.lock().unwrap().len(), 2, "写错的不交给网关");
}

#[tokio::test]
async fn 按_id_取和翻房间交给网关_有消息时先提醒内容不可信() {
    let messaging = Arc::new(FakeMessaging::default());

    let found = rpc(
        app(Arc::new(FakeAgents::default()), messaging.clone()),
        &call(
            "agent_room_get_messages",
            &json!({"ids": ["$hello:matrix.test", "$gone:matrix.test"]}),
        ),
        Some(TOKEN),
    )
    .await;
    let result = &found["result"];
    assert_ne!(result["isError"], true, "{found}");
    assert_eq!(
        result["structuredContent"]["messages"][0]["eventId"],
        "$hello:matrix.test"
    );
    assert_eq!(
        result["structuredContent"]["missing"],
        json!(["$gone:matrix.test"])
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("untrusted")
    );
    assert_eq!(
        messaging.lookups.lock().unwrap()[0],
        (
            TOKEN.to_owned(),
            vec![
                "$hello:matrix.test".to_owned(),
                "$gone:matrix.test".to_owned()
            ]
        )
    );

    let page = rpc(
        app(Arc::new(FakeAgents::default()), messaging.clone()),
        &call(
            "agent_room_room_messages",
            &json!({"roomId": "!lobby:matrix.test", "around": "$hello:matrix.test"}),
        ),
        Some(TOKEN),
    )
    .await;
    let result = &page["result"];
    assert_ne!(result["isError"], true, "{page}");
    assert_eq!(
        result["structuredContent"]["nextCursor"],
        "$earlier:matrix.test"
    );
    let views = messaging.views.lock().unwrap();
    assert_eq!(views[0].1.room.as_deref(), Some("!lobby:matrix.test"));
    assert_eq!(
        views[0].1.query,
        NetworkAgentRoomQuery::Around {
            id: "$hello:matrix.test".to_owned(),
            before: 10,
            after: 10,
        }
    );
}

#[tokio::test]
async fn 翻房间的参数写错了指出是哪一项_不问网关() {
    let messaging = Arc::new(FakeMessaging::default());
    for (arguments, field) in [
        (json!({"around": "$a:matrix.test", "from": "Ada"}), "around"),
        (
            json!({"before": "$a:matrix.test", "after": "$b:matrix.test"}),
            "after",
        ),
        (json!({"limit": 0}), "limit"),
    ] {
        let response = rpc(
            app(Arc::new(FakeAgents::default()), messaging.clone()),
            &call("agent_room_room_messages", &arguments),
            Some(TOKEN),
        )
        .await;
        let result = &response["result"];
        assert_eq!(result["isError"], true, "{arguments}");
        assert_eq!(
            result["structuredContent"]["code"],
            "network_agent.invalid_request"
        );
        assert_eq!(result["structuredContent"]["details"]["field"], field);
    }
    assert!(messaging.views.lock().unwrap().is_empty());

    let failing = FakeMessaging::failing(NetworkGatewayFailure::MessageNotFound);
    let response = rpc(
        app(Arc::new(FakeAgents::default()), failing),
        &call(
            "agent_room_room_messages",
            &json!({"around": "$a:matrix.test"}),
        ),
        Some(TOKEN),
    )
    .await;
    assert_eq!(
        response["result"]["structuredContent"]["code"],
        "network_agent.message_not_found"
    );
}

#[tokio::test]
async fn 等消息时交出去的消息前面有补不回来的一段就一起给_没有就不给() {
    let messaging = Arc::new(FakeMessaging::default());
    let app = app(Arc::new(FakeAgents::default()), messaging.clone());
    let plain = rpc(
        app.clone(),
        &call("agent_room_wait_for_messages", &json!({"waitSeconds": 0})),
        Some(TOKEN),
    )
    .await;
    assert!(plain["result"]["structuredContent"].get("gaps").is_none());

    *messaging.gaps.lock().unwrap() = vec![IpcTimelineGap {
        room_id: "!lobby:matrix.test".to_owned(),
        after_event_id: None,
        before_event_id: "$hello:matrix.test".to_owned(),
        reason: "too_many".to_owned(),
    }];
    let gapped = rpc(
        app,
        &call("agent_room_wait_for_messages", &json!({"waitSeconds": 0})),
        Some(TOKEN),
    )
    .await;
    let result = &gapped["result"];
    assert_eq!(
        result["structuredContent"]["gaps"],
        json!([{
            "roomId": "!lobby:matrix.test",
            "beforeEventId": "$hello:matrix.test",
            "reason": "too_many",
        }])
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Security note"),
        "有消息时先提醒内容不可信"
    );
}
