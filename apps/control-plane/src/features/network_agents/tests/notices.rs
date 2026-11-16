//! 房间的常驻公告：进房间的回答带这个房间现在的（`notice`），等消息的回答带改了的（`notices`）。

use std::sync::Arc;

use agent_room_application::ports::{MatrixRoomId, NetworkAgentRoomNotice};
use agent_room_bridge_ipc::{IpcPinUnreadable, IpcRoomPin, IpcRoomPins};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::json;
use tower::ServiceExt;

use super::{FakeAgents, FakeMessaging, TOKEN, app_with, body_json};

const LOBBY: &str = "!lobby:matrix.test";

pub(in crate::features::network_agents) fn notice(text: Option<&str>) -> NetworkAgentRoomNotice {
    NetworkAgentRoomNotice {
        room_id: MatrixRoomId::new(LOBBY).unwrap(),
        in_sub_room: None,
        text: text.map(str::to_owned),
        updated_by: "@owner:matrix.test".to_owned(),
        updated_at_ms: 1_793_692_801_000,
    }
}

/// 置顶了一句进来以前的消息的大厅。
pub(in crate::features::network_agents) fn pins() -> IpcRoomPins {
    IpcRoomPins {
        room_id: LOBBY.to_owned(),
        pinned: vec![IpcRoomPin {
            event_id: "$old:matrix.test".to_owned(),
            pinned_by: "@owner:matrix.test".to_owned(),
            pinned_at_unix_ms: 1_793_692_801_000,
            message: None,
            unreadable: Some(IpcPinUnreadable::BeforeJoin),
        }],
        updated_by: "@owner:matrix.test".to_owned(),
        updated_at_unix_ms: 1_793_692_801_000,
    }
}

fn app(messaging: &Arc<FakeMessaging>) -> axum::Router {
    app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
}

fn post(uri: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn wait() -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/v1/network-agents/me/messages?wait=0")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn 进房间的回答带上这个房间现在的公告_起名进房间也一样() {
    let messaging = Arc::new(FakeMessaging::default());
    let plain = app(&messaging)
        .oneshot(post("/v1/network-agents/me/rooms", r#"{"room":"general"}"#))
        .await
        .unwrap();
    assert!(
        body_json(plain).await.get("notice").is_none(),
        "没有公告不给"
    );

    *messaging.notice.lock().unwrap() = Some(notice(Some("只在被点名时说话。")));
    let entered = app(&messaging)
        .oneshot(post("/v1/network-agents/me/rooms", r#"{"room":"general"}"#))
        .await
        .unwrap();
    assert_eq!(entered.status(), StatusCode::OK);
    assert_eq!(
        body_json(entered).await["notice"],
        json!({
            "roomId": LOBBY,
            "text": "只在被点名时说话。",
            "updatedBy": "@owner:matrix.test",
            "updatedAtUnixMs": 1_793_692_801_000_i64,
        })
    );
    let created = app(&messaging)
        .oneshot(post("/v1/network-agents", r#"{"name":"Scout"}"#))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(
        body_json(created).await["notice"]["text"],
        "只在被点名时说话。"
    );
    let reads = messaging.notice_reads.lock().unwrap().clone();
    assert_eq!(reads.len(), 3);
    assert!(
        reads
            .iter()
            .all(|(token, room)| token == TOKEN && room == LOBBY),
        "拿新的令牌问进了的那个房间：{reads:?}"
    );
}

#[tokio::test]
async fn 等消息的回答带上改了的公告_撤掉的写明_没有就不给() {
    let messaging = Arc::new(FakeMessaging::default());
    let plain = app(&messaging).oneshot(wait()).await.unwrap();
    assert!(body_json(plain).await.get("notices").is_none());

    *messaging.changed_notices.lock().unwrap() = vec![notice(None)];
    let changed = app(&messaging).oneshot(wait()).await.unwrap();
    assert_eq!(
        body_json(changed).await["notices"],
        json!([{
            "roomId": LOBBY,
            "removed": true,
            "updatedBy": "@owner:matrix.test",
            "updatedAtUnixMs": 1_793_692_801_000_i64,
        }])
    );
}

/// 子房间里父房间那一段：`roomId` 是父房间的。
pub(in crate::features::network_agents) fn parent_notice(
    text: Option<&str>,
) -> NetworkAgentRoomNotice {
    NetworkAgentRoomNotice {
        room_id: MatrixRoomId::new("!project:matrix.test").unwrap(),
        in_sub_room: Some(MatrixRoomId::new(LOBBY).unwrap()),
        ..notice(text)
    }
}

#[tokio::test]
async fn 进子房间的回答另带父房间那一段_等消息时按父房间号给() {
    let messaging = Arc::new(FakeMessaging::default());
    *messaging.parent_notice.lock().unwrap() = Some(parent_notice(Some("先说要做什么。")));
    let entered = app(&messaging)
        .oneshot(post("/v1/network-agents/me/rooms", r#"{"room":"general"}"#))
        .await
        .unwrap();
    let body = body_json(entered).await;
    assert!(body.get("notice").is_none(), "这一间没有自己的：{body}");
    assert_eq!(
        body["parentNotice"],
        json!({
            "roomId": "!project:matrix.test",
            "text": "先说要做什么。",
            "updatedBy": "@owner:matrix.test",
            "updatedAtUnixMs": 1_793_692_801_000_i64,
        })
    );
    *messaging.notice.lock().unwrap() = Some(notice(Some("只改前端。")));
    let created = app(&messaging)
        .oneshot(post("/v1/network-agents", r#"{"name":"Scout"}"#))
        .await
        .unwrap();
    let body = body_json(created).await;
    assert_eq!(body["notice"]["text"], "只改前端。");
    assert_eq!(body["parentNotice"]["roomId"], "!project:matrix.test");

    *messaging.changed_notices.lock().unwrap() =
        vec![notice(Some("只改前端。")), parent_notice(None)];
    let changed = app(&messaging).oneshot(wait()).await.unwrap();
    assert_eq!(
        body_json(changed).await["notices"],
        json!([
            {
                "roomId": LOBBY,
                "text": "只改前端。",
                "updatedBy": "@owner:matrix.test",
                "updatedAtUnixMs": 1_793_692_801_000_i64,
            },
            {
                "roomId": "!project:matrix.test",
                "removed": true,
                "updatedBy": "@owner:matrix.test",
                "updatedAtUnixMs": 1_793_692_801_000_i64,
            }
        ])
    );
}

#[tokio::test]
async fn 进房间和等消息的回答带上置顶_没有就不给() {
    let messaging = Arc::new(FakeMessaging::default());
    let plain = app(&messaging)
        .oneshot(post("/v1/network-agents/me/rooms", r#"{"room":"general"}"#))
        .await
        .unwrap();
    assert!(body_json(plain).await.get("pins").is_none(), "没有置顶不给");

    *messaging.pins.lock().unwrap() = Some(pins());
    let entered = app(&messaging)
        .oneshot(post("/v1/network-agents/me/rooms", r#"{"room":"general"}"#))
        .await
        .unwrap();
    assert_eq!(
        body_json(entered).await["pins"],
        json!({
            "roomId": LOBBY,
            "pinned": [{
                "eventId": "$old:matrix.test",
                "pinnedBy": "@owner:matrix.test",
                "pinnedAtUnixMs": 1_793_692_801_000_i64,
                "unreadable": "before_join",
            }],
            "updatedBy": "@owner:matrix.test",
            "updatedAtUnixMs": 1_793_692_801_000_i64,
        })
    );
    let created = app(&messaging)
        .oneshot(post("/v1/network-agents", r#"{"name":"Scout"}"#))
        .await
        .unwrap();
    assert_eq!(
        body_json(created).await["pins"]["pinned"][0]["eventId"],
        "$old:matrix.test"
    );

    let plain = app(&messaging).oneshot(wait()).await.unwrap();
    assert!(body_json(plain).await.get("pins").is_none());
    *messaging.changed_pins.lock().unwrap() = vec![pins()];
    let changed = app(&messaging).oneshot(wait()).await.unwrap();
    assert_eq!(body_json(changed).await["pins"][0]["roomId"], LOBBY);
}
