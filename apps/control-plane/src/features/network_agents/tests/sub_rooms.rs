//! 网络 Agent 在它建的房间下面建子房间的 HTTP 接口（`specs/redesign/sub-rooms.md` 第 2h 步）：和建房
//! 同一个地址，带上 `parentRoomId` 就建子房间。

use std::sync::Arc;

use agent_room_application::network_agents::{NetworkAgentFailure, NetworkAgentFailureKind};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::json;
use tower::ServiceExt;

use super::{
    super::super::super::network_gateway::NetworkGatewayFailure, FakeAgents, FakeMessaging,
    PRIVATE_CATALOG_UUID, SUB_ROOM_CATALOG_UUID, TOKEN, app_with, body_json,
};

async fn create(messaging: &Arc<FakeMessaging>, body: &str) -> axum::response::Response {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/network-agents/me/private-rooms")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(request)
    .await
    .unwrap()
}

#[tokio::test]
async fn 带上父房间就建子房间_新建的回答_201_没有口令_带着父房间和分类() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = create(
        &messaging,
        &format!(
            r#"{{"name":"设计","parentRoomId":"{PRIVATE_CATALOG_UUID}","access":"invite_only","category":"进行中"}}"#
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    assert_eq!(
        body,
        json!({
            "schemaVersion": 1,
            "created": true,
            "room": {
                "catalogId": SUB_ROOM_CATALOG_UUID,
                "matrixRoomId": "!sub:matrix.test",
                "name": "设计",
            },
            "parent": {
                "catalogId": PRIVATE_CATALOG_UUID,
                "matrixRoomId": "!owned:matrix.test",
                "name": "周报",
            },
            "category": "进行中",
        })
    );
    assert_eq!(
        *messaging.created_sub_rooms.lock().unwrap(),
        [(
            TOKEN.to_owned(),
            format!("{PRIVATE_CATALOG_UUID} 设计 invite_only Some(\"进行中\")")
        )]
    );
    assert!(messaging.created_rooms.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 不给门就是没有门_回到同名子房间回答_200_分类没找到照实说() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = create(
        &messaging,
        r#"{"name":"已有的","parentRoomId":"!owned:matrix.test","category":"别的"}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["created"], false);
    assert_eq!(body["categoryMissing"], true);
    assert!(body.get("category").is_none());
    assert_eq!(
        messaging.created_sub_rooms.lock().unwrap()[0].1,
        "!owned:matrix.test 已有的 open Some(\"别的\")"
    );
}

#[tokio::test]
async fn 门和分类只跟父房间一起用_门不认识的也说清楚() {
    let messaging = Arc::new(FakeMessaging::default());
    for body in [
        r#"{"name":"设计","access":"open"}"#,
        r#"{"name":"设计","category":"进行中"}"#,
        r#"{"name":"设计","parentRoomId":"!owned:matrix.test","access":"knock"}"#,
    ] {
        let response = create(&messaging, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        let error = body_json(response).await;
        assert_eq!(error["code"], "network_agent.invalid_request");
        assert!(
            error["message"].as_str().unwrap().contains("parentRoomId"),
            "{error}"
        );
    }
    assert!(messaging.created_rooms.lock().unwrap().is_empty());
    assert!(messaging.created_sub_rooms.lock().unwrap().is_empty());
}

// ---- 看子房间名单（2h-2）

async fn sub_rooms(messaging: &Arc<FakeMessaging>, query: &str) -> axum::response::Response {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/v1/network-agents/me/private-rooms/{PRIVATE_CATALOG_UUID}/sub-rooms{query}"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(request)
    .await
    .unwrap()
}

#[tokio::test]
async fn 看子房间名单交给用例_有门又不在里面的没有人数() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = sub_rooms(&messaging, "?archived=true").await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body,
        json!({
            "schemaVersion": 1,
            "parent": {
                "catalogId": PRIVATE_CATALOG_UUID,
                "matrixRoomId": "!owned:matrix.test",
                "name": "周报",
            },
            "categories": ["进行中"],
            "subRooms": [
                {
                    "room": {
                        "catalogId": SUB_ROOM_CATALOG_UUID,
                        "matrixRoomId": "!sub:matrix.test",
                        "name": "设计",
                    },
                    "description": "画图",
                    "category": "进行中",
                    "access": "open",
                    "joined": true,
                    "createdByYou": true,
                    "people": 2,
                    "agents": 1,
                    "lastMessageAtUnixMs": 1_758_600_000_000_i64,
                },
                {
                    "room": {
                        "catalogId": "0198b601-77a1-7bb8-83eb-a8fe68c97e52",
                        "matrixRoomId": "!review:matrix.test",
                        "name": "评审",
                    },
                    "access": "request",
                    "joined": false,
                },
            ],
        })
    );
    assert_eq!(
        messaging.room_calls.lock().unwrap().clone(),
        [(
            TOKEN.to_owned(),
            format!("list_sub_rooms {PRIVATE_CATALOG_UUID} true")
        )]
    );
}

#[tokio::test]
async fn 看子房间名单的查询参数不对时_400_父房间用不了时照错误码回答() {
    let messaging = Arc::new(FakeMessaging::default());
    let wrong = sub_rooms(&messaging, "?archived=1").await;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(wrong).await["code"],
        "network_agent.invalid_request"
    );
    assert!(messaging.room_calls.lock().unwrap().is_empty());

    let unavailable = FakeMessaging::failing(NetworkGatewayFailure::Agent(
        NetworkAgentFailure::new(NetworkAgentFailureKind::SubRoomParentUnavailable),
    ));
    let response = sub_rooms(&unavailable, "").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(response).await["code"],
        "network_agent.sub_room_parent_unavailable"
    );
}
