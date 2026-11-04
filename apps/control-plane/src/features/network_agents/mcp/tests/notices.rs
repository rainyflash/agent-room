//! 远程 MCP 里的房间公告：进房间和等消息的结构化结果和 HTTP 一个形状，带着公告时第一段话先说
//! 公告是房间的规矩。

use std::sync::Arc;

use serde_json::{Value, json};

use super::super::super::tests::{FakeAgents, FakeMessaging, TOKEN, notices::notice};
use super::{app, call, rpc};

fn texts(response: &Value) -> Vec<&str> {
    response["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect()
}

#[tokio::test]
async fn 进房间和等消息带上公告_先说公告是房间的规矩() {
    let messaging = Arc::new(FakeMessaging::default());
    *messaging.notice.lock().unwrap() = Some(notice(Some("回复要短。")));
    *messaging.changed_notices.lock().unwrap() = vec![notice(Some("回复要短。"))];
    let app = app(Arc::new(FakeAgents::default()), messaging.clone());

    let joined = rpc(
        app.clone(),
        &call("agent_room_join", &json!({"name": "Scout"})),
        None,
    )
    .await;
    assert_eq!(
        joined["result"]["structuredContent"]["notice"]["text"],
        "回复要短。"
    );
    let said = texts(&joined);
    assert!(said[0].contains("Save the token"), "{said:?}");
    assert!(said[1].starts_with("This room has a notice"), "{said:?}");

    let entered = rpc(
        app.clone(),
        &call("agent_room_enter_room", &json!({"room": "general"})),
        Some(TOKEN),
    )
    .await;
    assert_eq!(
        entered["result"]["structuredContent"]["notice"]["roomId"],
        "!lobby:matrix.test"
    );
    assert!(texts(&entered)[0].starts_with("This room has a notice"));

    let waited = rpc(
        app,
        &call("agent_room_wait_for_messages", &json!({"waitSeconds": 0})),
        Some(TOKEN),
    )
    .await;
    assert_eq!(
        waited["result"]["structuredContent"]["notices"],
        json!([{
            "roomId": "!lobby:matrix.test",
            "text": "回复要短。",
            "updatedBy": "@owner:matrix.test",
            "updatedAtUnixMs": 1_793_692_801_000_i64,
        }])
    );
    assert!(texts(&waited)[0].starts_with("This room has a notice"));
}

#[tokio::test]
async fn 没有公告时进房间和等消息都不提() {
    let messaging = Arc::new(FakeMessaging::default());
    let app = app(Arc::new(FakeAgents::default()), messaging);
    let entered = rpc(
        app.clone(),
        &call("agent_room_enter_room", &json!({"room": "general"})),
        Some(TOKEN),
    )
    .await;
    assert!(
        entered["result"]["structuredContent"]
            .get("notice")
            .is_none()
    );
    let waited = rpc(
        app,
        &call("agent_room_wait_for_messages", &json!({"waitSeconds": 0})),
        Some(TOKEN),
    )
    .await;
    assert!(
        waited["result"]["structuredContent"]
            .get("notices")
            .is_none()
    );
    assert!(
        texts(&waited)
            .iter()
            .all(|text| !text.starts_with("This room has a notice"))
    );
}
