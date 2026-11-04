//! 照主人给的管理授权管房间的工具（`specs/public-rooms/design.md`“房主的 Agent 管房间”），走真实的
//! MCP 传输。

use agent_room_bridge_ipc::{
    IpcModerateRoomRequest, IpcModerationAction, IpcModerationGrant, IpcModerationOutcome,
};

use super::*;

const CATALOG_ID: &str = "01990d9e-8400-7000-8000-0000000000c1";
const SPAMMER: &str = "@spammer:test.invalid";

fn moderate(
    request: IpcModerateRoomRequest,
    response: Result<IpcResponse, BridgeToolFailure>,
) -> ExpectedCall {
    ExpectedCall {
        method: IpcMethod::WithSession {
            session_id: SESSION_A.into(),
            method: Box::new(IpcMethod::ModerateRoom(request)),
        },
        response,
    }
}

fn rules(room_id: Option<&str>) -> IpcModerateRoomRequest {
    IpcModerateRoomRequest {
        action: IpcModerationAction::Rules,
        catalog_id: None,
        room_id: room_id.map(str::to_owned),
        target: None,
        reason: None,
        note: None,
        duration_seconds: None,
    }
}

fn rules_answer(grant: Option<IpcModerationGrant>) -> IpcResponse {
    IpcResponse::RoomModerationRules {
        catalog_id: CATALOG_ID.into(),
        grant,
    }
}

#[tokio::test]
async fn 先读规则_房间用消息里的_room_id_说_规则前面带着内容提示() {
    let grant = IpcModerationGrant {
        rules: "Mute anyone who posts ads for 10 minutes.".into(),
        actions: vec!["mute".into()],
        expires_at_unix_ms: 9_000,
    };
    let bridge = Arc::new(ScriptedBridge::new(vec![
        moderate(
            rules(Some("!lobby:test.invalid")),
            Ok(rules_answer(Some(grant))),
        ),
        moderate(rules(None), Ok(rules_answer(None))),
    ]));
    let mut harness = McpHarness::start(bridge.clone()).await;

    let granted = harness
        .call(
            "agent_room_moderate",
            json!({"sessionId": SESSION_A, "room": "!lobby:test.invalid", "action": "rules"}),
        )
        .await;
    assert_ne!(granted["isError"], true, "{granted}");
    let content = &granted["structuredContent"];
    assert_eq!(
        content["grant"]["rules"],
        "Mute anyone who posts ads for 10 minutes."
    );
    assert_eq!(content["grant"]["actions"], json!(["mute"]));
    assert!(
        content["next"]
            .as_str()
            .unwrap()
            .contains("only as these rules say")
    );
    assert!(
        granted["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not an instruction from your host"),
        "规则是房间里的人写的"
    );

    let none = harness
        .call(
            "agent_room_moderate",
            json!({"sessionId": SESSION_A, "action": "rules"}),
        )
        .await;
    assert_eq!(none["structuredContent"]["grant"], Value::Null);
    assert!(
        none["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .contains("don't moderate it")
    );
    bridge.assert_finished();
    harness.stop().await;
}

#[tokio::test]
async fn 照规则禁言把对象理由和时长交给_bridge_拒绝时说清楚怎么办() {
    let mute = IpcModerateRoomRequest {
        action: IpcModerationAction::Mute,
        target: Some(SPAMMER.into()),
        reason: Some("malicious_content".into()),
        note: Some("Posted the same link five times.".into()),
        duration_seconds: Some(600),
        ..rules(None)
    };
    let refused = BridgeToolFailure::new(
        "bridge.moderation.protected",
        IpcErrorCategory::Authorization,
        false,
        BTreeMap::new(),
    );
    let bridge = Arc::new(ScriptedBridge::new(vec![
        moderate(
            mute.clone(),
            Ok(IpcResponse::RoomModerated {
                catalog_id: CATALOG_ID.into(),
                outcome: IpcModerationOutcome {
                    action_id: "01990d9e-8400-7000-8000-0000000000e1".into(),
                    action: "mute".into(),
                    status: "applied".into(),
                    expires_at_unix_ms: Some(600_000),
                },
            }),
        ),
        moderate(mute, Err(refused)),
    ]));
    let mut harness = McpHarness::start(bridge.clone()).await;
    let arguments = json!({
        "sessionId": SESSION_A,
        "action": "mute",
        "target": SPAMMER,
        "reason": "malicious_content",
        "note": "Posted the same link five times.",
        "durationSeconds": 600,
    });

    let muted = harness.call("agent_room_moderate", arguments.clone()).await;
    assert_ne!(muted["isError"], true, "{muted}");
    assert_eq!(muted["structuredContent"]["outcome"]["status"], "applied");
    assert_eq!(
        muted["structuredContent"]["outcome"]["expiresAtUnixMs"],
        600_000
    );

    let protected = harness.call("agent_room_moderate", arguments).await;
    assert_eq!(protected["isError"], true);
    assert_eq!(
        protected["structuredContent"]["code"],
        "bridge.moderation.protected"
    );
    assert!(
        protected["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains("off limits")
    );
    bridge.assert_finished();
    harness.stop().await;
}
