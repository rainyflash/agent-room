//! 远程 MCP 里网络 Agent 自己建房间、管房间的工具（`specs/agent-rooms/design.md` 第 4 步）：结构化结果
//! 和 HTTP 一个形状，第一段文字说下一步；建房和放行走网关，别的只动账。

use std::sync::Arc;

use agent_room_application::network_agents::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentRoomCode,
};
use serde_json::{Value, json};

use super::super::super::tests::{
    AGENT_UUID, FakeAgents, FakeMessaging, PRIVATE_CATALOG_UUID, TOKEN,
};
use super::{app, call, rpc};
use crate::network_gateway::NetworkGatewayFailure;

/// 建房以后拿到的房间：用它的 roomId 管房间。
const ROOM_ID: &str = "!owned:matrix.test";
const ACCOUNT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e60";

async fn tool(messaging: &Arc<FakeMessaging>, name: &str, arguments: &Value) -> Value {
    let response = rpc(
        app(Arc::new(FakeAgents::default()), messaging.clone()),
        &call(name, arguments),
        Some(TOKEN),
    )
    .await;
    response["result"].clone()
}

fn first_text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

fn calls(messaging: &FakeMessaging) -> Vec<String> {
    messaging
        .room_calls
        .lock()
        .unwrap()
        .drain(..)
        .map(|(token, call)| {
            assert_eq!(token, TOKEN);
            call
        })
        .collect()
}

#[tokio::test]
async fn 建房交给网关_结果和_http_一样_先说怎么请别的_agent_和人进来() {
    let messaging = Arc::new(FakeMessaging::default());
    let result = tool(
        &messaging,
        "agent_room_create_room",
        &json!({"name": "周报"}),
    )
    .await;

    let body = &result["structuredContent"];
    assert_eq!(body["created"], true);
    assert_eq!(
        body["room"],
        json!({"catalogId": PRIVATE_CATALOG_UUID, "matrixRoomId": ROOM_ID, "name": "周报"})
    );
    assert_eq!(
        body["code"],
        json!({"state": "visible", "code": "K7P3-Q9XW-2DMA"})
    );
    assert_eq!(
        body["guideUrl"],
        "https://api.agent-room.example/agents.txt"
    );
    assert!(
        body["share"]["zh"]
            .as_str()
            .unwrap()
            .contains("K7P3-Q9XW-2DMA")
    );
    assert!(
        body["share"]["en"]
            .as_str()
            .unwrap()
            .contains("K7P3-Q9XW-2DMA")
    );
    let text = first_text(&result);
    for part in [
        "You made the private room",
        "share",
        "agent_room_invite_person",
        "agent_room_answer_knock",
        "server can read everything",
    ] {
        assert!(text.contains(part), "缺 {part}：{text}");
    }
    assert_eq!(
        *messaging.created_rooms.lock().unwrap(),
        [(TOKEN.to_owned(), "周报".to_owned())]
    );

    let again = tool(
        &messaging,
        "agent_room_create_room",
        &json!({"name": "已有的"}),
    )
    .await;
    assert_eq!(again["structuredContent"]["created"], false);
    assert!(first_text(&again).starts_with("You already have an open room with this name"));
}

#[tokio::test]
async fn 看怎么进_没有口令就生成_看不到的不换_停用以后只能敲门_都列出敲门的() {
    let messaging = Arc::new(FakeMessaging::default());
    let access = |action: Option<&str>| {
        let mut arguments = json!({"roomId": ROOM_ID});
        if let Some(action) = action {
            arguments["action"] = json!(action);
        }
        arguments
    };

    let shown = tool(&messaging, "agent_room_room_access", &access(None)).await;
    assert_eq!(
        calls(&messaging),
        [
            format!("room_code {ROOM_ID}"),
            format!("waiting_knocks {ROOM_ID}")
        ]
    );
    let body = &shown["structuredContent"];
    assert_eq!(body["code"]["state"], "visible");
    assert_eq!(body["room"]["catalogId"], PRIVATE_CATALOG_UUID);
    assert_eq!(body["knocks"][0]["agentId"], AGENT_UUID);
    assert_eq!(body["knocks"][0]["displayName"], "Scout");
    let text = first_text(&shown);
    assert!(text.contains("pass on share") && text.contains("1 agent is knocking"));

    *messaging.room_code.lock().unwrap() = Some(NetworkAgentRoomCode::Missing);
    let generated = tool(&messaging, "agent_room_room_access", &access(Some("show"))).await;
    assert_eq!(
        calls(&messaging),
        [
            format!("room_code {ROOM_ID}"),
            format!("replace_code {ROOM_ID}"),
            format!("waiting_knocks {ROOM_ID}")
        ],
        "没有口令就生成一个"
    );
    assert_eq!(
        generated["structuredContent"]["code"]["code"],
        "N3WC-0DE2-XYZW"
    );

    *messaging.room_code.lock().unwrap() = Some(NetworkAgentRoomCode::Hidden);
    let hidden = tool(&messaging, "agent_room_room_access", &access(None)).await;
    assert_eq!(
        calls(&messaging),
        [
            format!("room_code {ROOM_ID}"),
            format!("waiting_knocks {ROOM_ID}")
        ],
        "看不到的不替它换"
    );
    assert_eq!(
        hidden["structuredContent"]["code"],
        json!({"state": "hidden"})
    );
    assert!(first_text(&hidden).contains("can't be shown"));

    *messaging.room_code.lock().unwrap() = Some(NetworkAgentRoomCode::Missing);
    let disabled = tool(
        &messaging,
        "agent_room_room_access",
        &access(Some("disable")),
    )
    .await;
    assert_eq!(
        calls(&messaging),
        [
            format!("disable_code {ROOM_ID}"),
            format!("room_code {ROOM_ID}"),
            format!("waiting_knocks {ROOM_ID}")
        ]
    );
    assert_eq!(
        disabled["structuredContent"]["code"],
        json!({"state": "missing"})
    );
    assert!(first_text(&disabled).contains("agents can only knock"));

    tool(
        &messaging,
        "agent_room_room_access",
        &access(Some("replace")),
    )
    .await;
    assert_eq!(
        calls(&messaging),
        [
            format!("replace_code {ROOM_ID}"),
            format!("waiting_knocks {ROOM_ID}")
        ]
    );
}

#[tokio::test]
async fn 回答敲门_放行走网关_不让进只动账_agent_id_不对先说清楚() {
    let messaging = Arc::new(FakeMessaging::default());
    let answer =
        |admit: bool, agent: &str| json!({"roomId": ROOM_ID, "agentId": agent, "admit": admit});

    let admitted = tool(
        &messaging,
        "agent_room_answer_knock",
        &answer(true, AGENT_UUID),
    )
    .await;
    assert_eq!(
        admitted["structuredContent"],
        json!({
            "schemaVersion": 1,
            "admitted": true,
            "agent": {"agentId": AGENT_UUID, "displayName": "Scout"},
        })
    );
    assert_eq!(
        calls(&messaging),
        [format!("admit_knock {ROOM_ID} {AGENT_UUID}")]
    );

    let declined = tool(
        &messaging,
        "agent_room_answer_knock",
        &answer(false, AGENT_UUID),
    )
    .await;
    assert_eq!(declined["structuredContent"]["admitted"], false);
    assert_eq!(
        calls(&messaging),
        [format!("decline_knock {ROOM_ID} {AGENT_UUID}")]
    );

    let wrong = tool(
        &messaging,
        "agent_room_answer_knock",
        &answer(true, "Scout"),
    )
    .await;
    assert_eq!(wrong["isError"], true);
    assert_eq!(
        wrong["structuredContent"]["code"],
        "network_agent.invalid_request"
    );
    assert!(first_text(&wrong).contains("incomingKnocks"));
    assert!(calls(&messaging).is_empty(), "agentId 不对就不问用例");
}

#[tokio::test]
async fn 邀请人_移出_agent_关房都交给用例_回答一句下一步() {
    let messaging = Arc::new(FakeMessaging::default());

    let invited = tool(
        &messaging,
        "agent_room_invite_person",
        &json!({"roomId": ROOM_ID, "accountId": ACCOUNT_UUID}),
    )
    .await;
    assert_eq!(invited["structuredContent"]["invited"], true);
    assert!(first_text(&invited).contains("server can read everything"));
    let not_an_account = tool(
        &messaging,
        "agent_room_invite_person",
        &json!({"roomId": ROOM_ID, "accountId": "someone@example.com"}),
    )
    .await;
    assert_eq!(
        not_an_account["structuredContent"]["code"],
        "network_agent.invalid_request"
    );

    let removed = tool(
        &messaging,
        "agent_room_remove_agent",
        &json!({"roomId": ROOM_ID, "agentId": AGENT_UUID}),
    )
    .await;
    assert_eq!(removed["structuredContent"]["removed"], true);

    let closed = tool(
        &messaging,
        "agent_room_close_room",
        &json!({"roomId": PRIVATE_CATALOG_UUID}),
    )
    .await;
    assert_eq!(closed["structuredContent"]["closed"], true);
    assert!(first_text(&closed).contains("can't be undone"));

    assert_eq!(
        calls(&messaging),
        [
            format!("invite_person {ROOM_ID} {ACCOUNT_UUID}"),
            format!("remove_agent {ROOM_ID} {AGENT_UUID}"),
            format!("close_room {PRIVATE_CATALOG_UUID}"),
        ]
    );
}

#[tokio::test]
async fn 管别人的房间照_http_的错误码回答() {
    let messaging = FakeMessaging::failing(NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
        NetworkAgentFailureKind::RoomNotOwned,
    )));
    let result = tool(
        &messaging,
        "agent_room_close_room",
        &json!({"roomId": "!someone-else:matrix.test"}),
    )
    .await;
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["code"],
        "network_agent.room_not_owned"
    );
    assert!(first_text(&result).contains("agent_room_get_self"));
}
